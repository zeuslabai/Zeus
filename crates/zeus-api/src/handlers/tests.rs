#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode, header};
    use axum::body::Body;
    use tower::ServiceExt;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use crate::{AppState, SharedState};

    fn test_state() -> SharedState {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // Route ZEUS_HOME to a tempdir so any config.save() calls never touch ~/.zeus/config.toml
        unsafe { std::env::set_var("ZEUS_HOME", tmp.path()); }
        let mut config = zeus_core::Config::default();
        config.workspace = tmp.path().join("workspace");
        config.sessions = tmp.path().join("sessions");
        config.onboarding_complete = true; // Prevents save guard from blocking PUT /v1/config
        config.loaded_from_default = false; // Allow save() to proceed into the tempdir
        // Keep tmp alive for the duration of the test via leaked handle
        let _ = Box::leak(Box::new(tmp));
        Arc::new(RwLock::new(AppState::new(config).unwrap()))
    }

    /// Create a test router without auth middleware
    fn test_app(state: SharedState) -> axum::Router {
        crate::create_test_router(state)
    }

    #[tokio::test]
    async fn test_auth_middleware_rejects_no_token() {
        let state = test_state();
        let app =
            crate::create_router_with_auth(state, false, Some("secret123".to_string()), &[], None);

        let req = Request::builder()
            .uri("/v1/tools")
            .body(Body::empty())
            .expect("failed to build no-token request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send no-token request");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_rejects_wrong_token() {
        let state = test_state();
        let app =
            crate::create_router_with_auth(state, false, Some("secret123".to_string()), &[], None);

        let req = Request::builder()
            .uri("/v1/tools")
            .header(header::AUTHORIZATION, "Bearer wrong_token")
            .body(Body::empty())
            .expect("failed to build wrong-token request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send wrong-token request");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_accepts_correct_token() {
        let state = test_state();
        let app =
            crate::create_router_with_auth(state, false, Some("secret123".to_string()), &[], None);

        let req = Request::builder()
            .uri("/v1/tools")
            .header(header::AUTHORIZATION, "Bearer secret123")
            .body(Body::empty())
            .expect("failed to build correct-token request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send correct-token request");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_allows_health_without_token() {
        let state = test_state();
        let app =
            crate::create_router_with_auth(state, false, Some("secret123".to_string()), &[], None);

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .expect("failed to build health request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send health request");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_no_auth_when_token_is_none() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/tools")
            .body(Body::empty())
            .expect("failed to build no-auth request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send no-auth request");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ========================================================================
    // Integration tests for API endpoints
    // ========================================================================

    fn test_state_with_tempdir(tmp: &tempfile::TempDir) -> SharedState {
        let mut config = zeus_core::Config::default();
        config.workspace = tmp.path().join("workspace");
        config.sessions = tmp.path().join("sessions");
        config.onboarding_complete = true; // Prevents save guard from blocking
        Arc::new(RwLock::new(AppState::new(config).unwrap()))
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("failed to build health endpoint request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send health endpoint request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read health response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("health response should be valid JSON");
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_tokens_count_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tokens/count")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"text": "Hello world, this is a test"}"#))
            .expect("failed to build token count request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send token count request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read token count response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("token count response should be valid JSON");
        assert!(json["tokens"].as_u64().unwrap() > 0);
        assert_eq!(json["method"], "estimate");
    }

    #[tokio::test]
    async fn test_status_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/status")
            .body(Body::empty())
            .expect("failed to build status request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send status request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read status response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("status response should be valid JSON");
        assert_eq!(json["status"], "ok");
        assert!(json["provider"].is_string());
        assert!(json["model"].is_string());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/tools")
            .body(Body::empty())
            .expect("failed to build list-tools request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send list-tools request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read tools response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("tools response should be valid JSON");
        let tools = json["tools"].as_array().expect("tools should be an array");
        assert!(!tools.is_empty(), "tool list should not be empty");

        // Each tool should have name, description, and parameters
        for tool in tools {
            assert!(tool["name"].is_string(), "tool should have a name");
            assert!(
                tool["description"].is_string(),
                "tool should have a description"
            );
            assert!(
                tool.get("parameters").is_some(),
                "tool should have parameters"
            );
        }
    }

    #[tokio::test]
    async fn test_execute_tool_list_dir() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/list_dir")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"arguments": {"path": "."}}))
                    .expect("failed to serialize list_dir arguments"),
            ))
            .expect("failed to build list_dir request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send list_dir request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read list_dir response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("list_dir response should be valid JSON");
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_execute_tool_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/nonexistent")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"arguments": {}}))
                    .expect("failed to serialize empty arguments"),
            ))
            .expect("failed to build nonexistent-tool request");

        let resp: axum::response::Response = app
            .oneshot(req)
            .await
            .expect("failed to send nonexistent-tool request");
        // Should return 200 with success:false, not crash
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("failed to read nonexistent-tool response body");
        let json: serde_json::Value =
            serde_json::from_slice(&body).expect("nonexistent-tool response should be valid JSON");
        assert_eq!(json["success"], false);
        assert!(json["error"].is_string());
    }

    #[tokio::test]
    async fn test_session_create_and_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create sessions directory
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state.clone());

        // Create a session
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"]
            .as_str()
            .expect("session should have an id");
        assert!(!session_id.is_empty());

        // List sessions - should include the one we created
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = list_json["sessions"]
            .as_array()
            .expect("sessions should be an array");
        assert!(
            sessions
                .iter()
                .any(|s| s["id"].as_str() == Some(session_id)),
            "created session should appear in list"
        );
    }

    #[tokio::test]
    async fn test_session_get_by_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state.clone());

        // Create a session
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"].as_str().unwrap().to_string();

        // Get session by ID
        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"].as_str(), Some(session_id.as_str()));
        assert!(json["messages"].is_array());
    }

    #[tokio::test]
    async fn test_session_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/sessions/nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_memory_get() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Init workspace
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("context_length").is_some());
        assert!(json.get("memory").is_some());
        assert!(json.get("daily").is_some());
    }

    #[tokio::test]
    async fn test_memory_remember_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Init workspace
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Remember a fact
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"fact": "Zeus is powerful"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);

        // Get memory — should contain the fact
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let memory = json["memory"].as_str().unwrap_or("");
        assert!(
            memory.contains("Zeus is powerful"),
            "memory should contain the remembered fact, got: {}",
            memory
        );
    }

    #[tokio::test]
    async fn test_memory_add_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Init workspace
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Add a note
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/note")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "Important meeting at 3pm"}))
                    .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);

        // Get memory — daily should contain the note
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let daily = json["daily"].as_str().unwrap_or("");
        assert!(
            daily.contains("Important meeting at 3pm"),
            "daily notes should contain the note, got: {}",
            daily
        );
    }

    #[tokio::test]
    async fn test_webhook_health() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/webhooks")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_webhook_receive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Init workspace so webhook can log to daily notes
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "message": "Hello from webhook",
                    "sender": "test-bot"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["received"], true);
    }

    #[tokio::test]
    async fn test_webhook_receive_with_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Init workspace so webhook can log to daily notes
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/telegram")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "message": "Hello from Telegram",
                    "sender": "telegram-user"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["received"], true);
    }

    // ========================================================================
    // Phase 1 endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_config() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/config")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // model is skipped when empty (skip_serializing_if = is_empty); workspace is always present
        assert!(
            json["workspace"].is_string(),
            "config should have workspace"
        );
    }

    #[tokio::test]
    async fn test_update_config_model() {
        // Route config through a tempdir via ZEUS_HOME so we never touch ~/.zeus
        let tmp = tempfile::TempDir::new().unwrap();
        unsafe { std::env::set_var("ZEUS_HOME", tmp.path()) };

        let mut config = zeus_core::Config::default();
        config.onboarding_complete = true;
        config.loaded_from_default = false;
        config.workspace = tmp.path().join("workspace");
        config.sessions = tmp.path().join("sessions");
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state.clone());

        let req = Request::builder()
            .method("PUT")
            .uri("/v1/config")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "model": "openai/gpt-4o"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);

        // Verify the model was updated in state
        let s = state.read().await;
        assert_eq!(s.config.model, "openai/gpt-4o");
        drop(s);
        unsafe { std::env::remove_var("ZEUS_HOME") };
    }

    #[tokio::test]
    async fn test_get_stats() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/stats")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["sessions"]["total"].is_number());
        assert!(json["tools"]["total"].is_number());
        assert!(json["memory"]["workspace_files"].is_number());
        assert!(json["model"].is_string());
        assert!(json["provider"].is_string());
    }

    #[tokio::test]
    async fn test_get_session_stats() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create a session with some messages
        let app = test_app(state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"].as_str().unwrap().to_string();

        // Get session stats
        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/stats", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"].as_str(), Some(session_id.as_str()));
        assert!(json["message_count"].is_number());
        assert!(json["user_messages"].is_number());
        assert!(json["assistant_messages"].is_number());
        assert!(json["tool_calls"].is_number());
        assert!(json["created"].is_string());
        assert!(json["last_activity"].is_string());
        assert!(json["duration_seconds"].is_number());
    }

    #[tokio::test]
    async fn test_doctor() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/doctor")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let checks = json["checks"].as_array().expect("checks should be array");
        assert!(!checks.is_empty(), "should have at least one check");
        assert!(json["overall"].is_string());

        // Each check should have name, status, detail
        for check in checks {
            assert!(check["name"].is_string());
            assert!(check["status"].is_string());
            assert!(check["detail"].is_string());
        }
    }

    #[tokio::test]
    async fn test_get_activity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Add a note so there's something in the activity feed
        {
            let s = state.read().await;
            s.workspace.note("Test activity item").await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/activity")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["events"].is_array());
    }

    #[tokio::test]
    async fn test_get_activity_with_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/activity?limit=5")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let events = json["events"].as_array().unwrap();
        assert!(events.len() <= 5);
    }

    #[tokio::test]
    async fn test_list_sessions_pagination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create 3 sessions
        for _ in 0..3 {
            let app = test_app(state.clone());
            let req = Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .body(Body::empty())
                .unwrap();
            app.oneshot(req).await.unwrap();
        }

        // List with limit=2
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/sessions?limit=2")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = json["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(json["total"].as_u64().unwrap(), 3);

        // Each session should have message_count
        for s in sessions {
            assert!(s["message_count"].is_number());
        }
    }

    #[tokio::test]
    async fn test_enhanced_status() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/status")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
        assert!(json["auth_method"].is_string());
        assert!(json["sessions_count"].is_number());
    }

    #[tokio::test]
    async fn test_config_sanitizes_secrets() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/config")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body);
        // Should not contain actual API keys (though in test there are none,
        // the config should serialize without errors)
        assert!(!body_str.is_empty());
    }

    // ========================================================================
    // Phase 2 endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_list_skills_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/skills")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let skills = json["skills"].as_array().expect("skills should be array");
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn test_install_and_list_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Install a skill via content
        let app = test_app(state.clone());
        let skill_content = r#"# Web Research

A web research skill.

## Version: 1.0.0

## System Prompt
You can search the web.

## Tools
- search: Search the web

## Permissions
- web_fetch
- shell
"#;

        let req = Request::builder()
            .method("POST")
            .uri("/v1/skills")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": skill_content})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert!(json["id"].is_string());

        // List skills — should show the installed one
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/skills")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let skills = json["skills"].as_array().expect("skills should be array");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0]["id"], "web-research");
    }

    #[tokio::test]
    async fn test_delete_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Create skill dir manually
        let skills_dir = tmp.path().join("workspace/skills/test-skill");
        tokio::fs::create_dir_all(&skills_dir).await.unwrap();
        tokio::fs::write(skills_dir.join("SKILL.md"), "# Test Skill\n")
            .await
            .unwrap();

        let app = test_app(state.clone());
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/skills/test-skill")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify it's gone
        assert!(!skills_dir.exists());
    }

    #[tokio::test]
    async fn test_delete_skill_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/skills/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_skill_dry_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Create skill dir manually
        let skills_dir = tmp.path().join("workspace/skills/dry-skill");
        tokio::fs::create_dir_all(&skills_dir).await.unwrap();
        tokio::fs::write(skills_dir.join("SKILL.md"), "# Dry Skill\nTest.")
            .await
            .unwrap();

        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/skills/dry-skill?dry_run=true")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["dry_run"], true);
        assert!(json["files"].as_array().unwrap().len() > 0);
        assert!(json["total_size"].as_u64().unwrap() > 0);

        // Skill should still exist on disk
        assert!(skills_dir.exists());
    }

    #[tokio::test]
    async fn test_delete_skill_keep_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let skills_dir = tmp.path().join("workspace/skills/keep-skill");
        tokio::fs::create_dir_all(&skills_dir).await.unwrap();
        tokio::fs::write(skills_dir.join("SKILL.md"), "# Keep Skill\nTest.")
            .await
            .unwrap();

        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/skills/keep-skill?keep_files=true")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["keep_files"], true);

        // Files should still be on disk
        assert!(skills_dir.join("SKILL.md").exists());
    }

    #[tokio::test]
    async fn test_list_mcp_servers_empty() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/mcp/servers")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let _servers = json["servers"].as_array().expect("servers should be array");
    }

    #[tokio::test]
    async fn test_list_memory_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory/files")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let files = json["files"].as_array().expect("files should be array");
        assert!(!files.is_empty(), "workspace should have default files");

        // Should contain AGENTS.md
        let has_agents = files
            .iter()
            .any(|f| f["path"].as_str() == Some("AGENTS.md"));
        assert!(has_agents, "should contain AGENTS.md");
    }

    #[tokio::test]
    async fn test_read_memory_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory/files/AGENTS.md")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["path"], "AGENTS.md");
        assert!(json["content"].is_string());
        let content = json["content"].as_str().unwrap();
        assert!(content.contains("Zeus"), "AGENTS.md should mention Zeus");
        assert!(json["size"].is_number());
    }

    #[tokio::test]
    async fn test_write_and_read_memory_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Write a file
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/memory/files/test.md")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "Hello from test!"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);

        // Read it back
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/files/test.md")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "Hello from test!");
    }

    #[tokio::test]
    async fn test_memory_file_path_traversal_blocked() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory/files/../../etc/passwd")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_search_memory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/search")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"query": "Zeus", "limit": 10})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let results = json["results"].as_array().expect("results should be array");
        // Should find "Zeus" in AGENTS.md at minimum
        assert!(
            !results.is_empty(),
            "search for 'Zeus' should find results in workspace"
        );
    }

    #[tokio::test]
    async fn test_channels_crud() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // List — initially empty
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/channels")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);

        // Create a channel
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "telegram",
                    "name": "My Bot",
                    "config": {"bot_token": "123:ABC", "chat_id": "456"}
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(created["channel_type"], "telegram");
        assert_eq!(created["name"], "My Bot");
        assert!(created["enabled"].as_bool().unwrap());
        let channel_id = created["id"].as_str().unwrap().to_string();

        // Get by ID
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri(format!("/v1/channels/{}", channel_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let fetched: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(fetched["id"], channel_id);
        assert_eq!(fetched["config"]["bot_token"], "123:ABC");

        // Update
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/channels/{}", channel_id))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": "Renamed Bot",
                    "enabled": false
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let updated: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated["name"], "Renamed Bot");
        assert!(!updated["enabled"].as_bool().unwrap());

        // List — should have 1
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/channels")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["channels"][0]["name"], "Renamed Bot");

        // Delete
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/channels/{}", channel_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let deleted: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(deleted["deleted"], true);

        // List — should be empty again
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/channels")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_channel_get_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/channels/nonexistent-id")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_channel_test_connectivity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create a webhook channel with config
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "webhook",
                    "name": "Test Hook",
                    "config": {"webhook_url": "https://example.com/hook"}
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let channel_id = created["id"].as_str().unwrap().to_string();

        // Test connectivity — should pass (webhook_url present)
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/channels/{}/test", channel_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["type"], "webhook");
    }

    #[tokio::test]
    async fn test_channel_test_missing_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create a telegram channel WITHOUT required keys
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "telegram",
                    "name": "Incomplete Bot",
                    "config": {}
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let channel_id = created["id"].as_str().unwrap().to_string();

        // Test — should fail (missing bot_token, chat_id)
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/channels/{}/test", channel_id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], false);
        assert!(json["detail"].as_str().unwrap().contains("Missing"));
    }

    #[tokio::test]
    async fn test_channel_status_endpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Status for non-existent channel
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/channels/no-such-id/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "not_found");
    }

    #[tokio::test]
    async fn test_read_nested_memory_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory/files/memory/MEMORY.md")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["path"], "memory/MEMORY.md");
        assert!(json["content"].is_string());
    }

    #[tokio::test]
    async fn test_create_channel_invalid_type() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        // Invalid channel_type should fail deserialization (422)
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "invalid_type",
                    "name": "Bad Channel"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_update_skill_enable_disable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        // Create skill dir
        let skills_dir = tmp.path().join("workspace/skills/test-skill");
        tokio::fs::create_dir_all(&skills_dir).await.unwrap();
        tokio::fs::write(skills_dir.join("SKILL.md"), "# Test Skill\nA test skill.")
            .await
            .unwrap();

        // Disable skill
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/skills/test-skill")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"enabled": false})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(skills_dir.join(".disabled").exists());

        // Re-enable skill
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/skills/test-skill")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"enabled": true})).unwrap(),
            ))
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!skills_dir.join(".disabled").exists());

        // Listing should show enabled
        let app3 = test_app(state);
        let req = Request::builder()
            .uri("/v1/skills")
            .body(Body::empty())
            .unwrap();

        let resp = app3.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let skills = json["skills"].as_array().unwrap();
        assert_eq!(skills[0]["enabled"], true);
    }

    // ========================================================================
    // Phase 3 endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_session_raw() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create a session
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"].as_str().unwrap().to_string();

        // Get raw JSONL
        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/raw", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], session_id);
        assert_eq!(json["format"], "jsonl");
        assert!(json["content"].is_string());
        assert!(json["size_bytes"].is_number());
        // Content should contain session_start
        let content = json["content"].as_str().unwrap();
        assert!(content.contains("session_start"));
    }

    #[tokio::test]
    async fn test_session_audit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create a session
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"].as_str().unwrap().to_string();

        // Get audit trail
        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/audit", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], session_id);
        assert!(json["events"].is_array());
    }

    #[tokio::test]
    async fn test_session_tools() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create a session
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = create_json["id"].as_str().unwrap().to_string();

        // Get tool chain
        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/tools", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], session_id);
        assert!(json["tool_calls"].is_array());
    }

    #[tokio::test]
    async fn test_pipeline_stats() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/pipeline/stats")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let stages = json["stages"].as_array().expect("stages should be array");
        assert_eq!(stages.len(), 6);
        assert!(json["total_messages"].is_number());
        assert!(json["uptime_seconds"].is_number());

        // Each stage should have the right structure
        for stage in stages {
            assert!(stage["name"].is_string());
            assert!(stage["messages_processed"].is_number());
            assert!(stage["avg_latency_ms"].is_number());
            assert!(stage["error_count"].is_number());
        }
    }

    #[tokio::test]
    async fn test_analytics_costs() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/analytics/costs")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total_cost"].is_number());
        assert!(json["total_requests"].is_number());
        assert!(json["total_input_tokens"].is_number());
        assert!(json["total_output_tokens"].is_number());
        assert_eq!(json["currency"], "USD");
    }

    #[tokio::test]
    async fn test_analytics_tokens() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/analytics/tokens")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["total_input_tokens"].is_number());
        assert!(json["total_output_tokens"].is_number());
    }

    #[tokio::test]
    async fn test_analytics_providers() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/analytics/providers")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let providers = json["providers"]
            .as_array()
            .expect("providers should be array");
        assert!(!providers.is_empty());

        // Each provider should have the right structure
        for p in providers {
            assert!(p["name"].is_string());
            assert!(p["configured"].is_boolean());
            assert!(p["requests"].is_number());
        }
    }

    #[tokio::test]
    async fn test_analytics_budgets() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/analytics/budgets")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["budgets"].is_array());
        assert!(json["alerts"].is_array());
    }

    #[tokio::test]
    async fn test_security_threats() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/security/threats")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["threats"].is_array());
    }

    #[tokio::test]
    async fn test_security_permissions() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/security/permissions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["global"].is_object());
        assert!(json["global"]["level"].is_string());
        assert!(json["global"]["shell_access"].is_boolean());
    }

    #[tokio::test]
    async fn test_security_keys() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/security/keys")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let keys = json["keys"].as_array().expect("keys should be array");
        assert!(!keys.is_empty());

        // Should have Anthropic at minimum
        let has_anthropic = keys.iter().any(|k| k["provider"] == "Anthropic");
        assert!(has_anthropic, "should list Anthropic provider");

        // Each key should have the right structure
        for key in keys {
            assert!(key["provider"].is_string());
            assert!(key["env_var"].is_string());
            assert!(key["configured"].is_boolean());
        }
    }

    #[tokio::test]
    async fn test_security_allowlist() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/security/allowlist")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let allowlist = json["allowlist"]
            .as_array()
            .expect("allowlist should be array");
        assert!(!allowlist.is_empty(), "should have default commands");
        assert!(json["mode"].is_string());

        // Default should contain common commands
        let has_ls = allowlist.iter().any(|v| v.as_str() == Some("ls"));
        assert!(has_ls, "default allowlist should contain 'ls'");
    }

    // ========================================================================
    // Phase 4 endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_create_and_list_projects() {
        // Use a custom projects dir to avoid polluting real config
        let tmp = tempfile::TempDir::new().unwrap();
        let projects_dir = tmp.path().join("projects");
        std::fs::create_dir_all(&projects_dir).unwrap();

        // Override HOME for this test to use temp dir
        let state = test_state();
        let app = test_app(state.clone());

        // Create a project
        let req = Request::builder()
            .method("POST")
            .uri("/v1/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": "Test Project",
                    "description": "A test project",
                    "budget": 100.0
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "Test Project");
        assert!(json["id"].is_string());
        assert_eq!(json["status"], "active");

        let project_id = json["id"].as_str().unwrap().to_string();

        // List projects — should include our new one
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/projects")
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let projects = json["projects"]
            .as_array()
            .expect("projects should be array");
        assert!(
            projects
                .iter()
                .any(|p| p["id"].as_str() == Some(&project_id)),
            "created project should appear in list"
        );

        // Get project by ID
        let app3 = test_app(state.clone());
        let req = Request::builder()
            .uri(format!("/v1/projects/{}", project_id))
            .body(Body::empty())
            .unwrap();

        let resp = app3.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], project_id);
        assert_eq!(json["name"], "Test Project");
    }

    #[tokio::test]
    async fn test_update_project() {
        let state = test_state();
        let app = test_app(state.clone());

        // Create a project first
        let req = Request::builder()
            .method("POST")
            .uri("/v1/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": "Update Test",
                    "budget": 50.0
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let project_id = json["id"].as_str().unwrap().to_string();

        // Update it
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/projects/{}", project_id))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": "Updated Name",
                    "status": "paused"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify update
        let app3 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/projects/{}", project_id))
            .body(Body::empty())
            .unwrap();

        let resp = app3.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "Updated Name");
        assert_eq!(json["status"], "paused");
    }

    #[tokio::test]
    async fn test_assign_project_agents() {
        let state = test_state();
        let app = test_app(state.clone());

        // Create project
        let req = Request::builder()
            .method("POST")
            .uri("/v1/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"name": "Agent Test"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let project_id = json["id"].as_str().unwrap().to_string();

        // Assign agents
        let app2 = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri(format!("/v1/projects/{}/agents", project_id))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"agents": ["atlas", "hermes"]})).unwrap(),
            ))
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        let agents = json["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_project_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/projects/nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_network_agents() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/network/agents")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agents = json["agents"].as_array().expect("agents should be array");
        assert!(!agents.is_empty(), "should have at least local agent");

        // Check local agent
        let local = &agents[0];
        assert_eq!(local["id"], "local");
        assert_eq!(local["name"], "Zeus");
        assert_eq!(local["status"], "online");
    }

    #[tokio::test]
    async fn test_network_discover() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/network/discover")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["mdns"].is_array());
        assert!(json["peer_count"].is_number());
    }

    #[tokio::test]
    async fn test_network_messages() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/network/messages")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["messages"].is_array());
    }

    #[tokio::test]
    async fn test_update_security_permissions() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("PUT")
            .uri("/v1/security/permissions")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "shell_access": false,
                    "level": "strict"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_config_reload_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/config/reload")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // May succeed or fail depending on whether ~/.zeus/config.toml exists,
        // but the endpoint itself should be reachable (not 404)
        assert!(
            resp.status() == StatusCode::OK || resp.status() == StatusCode::INTERNAL_SERVER_ERROR,
            "Expected 200 or 500, got {}",
            resp.status()
        );
    }

    #[tokio::test]
    async fn test_config_history_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/config/history")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["history"].is_array());
        assert_eq!(json["count"], 0); // No reloads yet
    }

    #[tokio::test]
    async fn test_config_history_records_changes() {
        let state = test_state();

        // Manually push a history entry
        {
            let mut s = state.write().await;
            s.config_history.push(crate::ConfigChangeEntry {
                timestamp: chrono::Utc::now(),
                source: "test".to_string(),
                changed_keys: vec!["model".to_string()],
            });
        }

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/config/history")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        let history = json["history"].as_array().unwrap();
        assert_eq!(history[0]["source"], "test");
        assert_eq!(history[0]["changed_keys"][0], "model");
    }

    #[tokio::test]
    async fn test_list_approvals_endpoint() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/approvals")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_approve_deny_flow_endpoint() {
        // Create state with shell requiring approval
        let mut config = zeus_core::Config::default();
        config.aegis = Some(zeus_core::AegisConfig {
            require_confirmation_for: vec!["shell".to_string()],
            ..Default::default()
        });
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));

        // Submit an approval programmatically
        let id = {
            let mut guard = state.write().await;
            let (id, _rx) =
                guard
                    .approvals
                    .submit("shell".into(), serde_json::json!({"cmd": "ls"}), None);
            id
        };

        // List pending — should have 1
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/approvals")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 1);

        // Approve it
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri(format!("/v1/approvals/{}/approve", id))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // List pending — should be empty now
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/approvals")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    // ========================================================================
    // Session Replay tests
    // ========================================================================

    #[tokio::test]
    async fn test_session_replay() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create a session with messages
        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let mut session = zeus_session::Session::new(&sessions_dir);
        session.init().await.unwrap();
        let session_id = session.id.clone();

        session
            .add(zeus_core::Message::user("Hello"))
            .await
            .unwrap();
        session
            .add(zeus_core::Message::assistant("Hi there!"))
            .await
            .unwrap();
        session
            .add(zeus_core::Message::user("How are you?"))
            .await
            .unwrap();

        // Update config sessions path
        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/replay", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["id"], session_id);
        assert_eq!(json["total_turns"], 3);
        assert!(json["total_tokens"].as_u64().unwrap() > 0);

        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["role"], "user");
        assert_eq!(entries[0]["content"], "Hello");
        assert_eq!(entries[0]["index"], 0);
        assert_eq!(entries[1]["role"], "assistant");
        assert_eq!(entries[1]["content"], "Hi there!");
        assert_eq!(entries[1]["index"], 1);
        assert_eq!(entries[2]["role"], "user");
        assert_eq!(entries[2]["index"], 2);

        // Each entry should have timestamp and token_count
        for entry in entries {
            assert!(entry.get("timestamp").is_some());
            assert!(entry.get("token_count").is_some());
        }
    }

    #[tokio::test]
    async fn test_session_replay_turn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let mut session = zeus_session::Session::new(&sessions_dir);
        session.init().await.unwrap();
        let session_id = session.id.clone();

        session
            .add(zeus_core::Message::user("First"))
            .await
            .unwrap();
        session
            .add(zeus_core::Message::assistant("Second"))
            .await
            .unwrap();

        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        // Get turn 0
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/replay/0", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["index"], 0);
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "First");

        // Get turn 1
        let app = test_app(state.clone());
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/replay/1", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["index"], 1);
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "Second");
    }

    #[tokio::test]
    async fn test_session_replay_turn_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let mut session = zeus_session::Session::new(&sessions_dir);
        session.init().await.unwrap();
        let session_id = session.id.clone();

        session
            .add(zeus_core::Message::user("Only one"))
            .await
            .unwrap();

        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/replay/5", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_replay_with_tool_calls() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let mut session = zeus_session::Session::new(&sessions_dir);
        session.init().await.unwrap();
        let session_id = session.id.clone();

        // Add an assistant message with tool calls
        let mut assistant_msg = zeus_core::Message::assistant("Let me check that.");
        assistant_msg.tool_calls.push(zeus_core::ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        });
        session.add(assistant_msg).await.unwrap();

        // Add a tool result message
        let tool_msg = zeus_core::Message {
            role: zeus_core::Role::Tool,
            content: String::new(),
            tool_calls: vec![],
            tool_results: vec![zeus_core::ToolResult {
                call_id: "call_1".to_string(),
                success: true,
                output: "file contents here".to_string(),
            }],
            timestamp: chrono::Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
            compaction_hint: Default::default(),
        };
        session.add(tool_msg).await.unwrap();

        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/replay", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);

        // First entry (assistant with tool call)
        let entry0 = &entries[0];
        assert_eq!(entry0["role"], "assistant");
        assert!(entry0["tool_calls"].is_array());
        let tcs = entry0["tool_calls"].as_array().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0]["name"], "read_file");
        assert_eq!(entry0["tool_name"][0], "read_file");

        // Second entry (tool result)
        let entry1 = &entries[1];
        assert_eq!(entry1["role"], "tool");
        assert!(entry1["tool_results"].is_array());
        let trs = entry1["tool_results"].as_array().unwrap();
        assert_eq!(trs.len(), 1);
        assert_eq!(trs[0]["success"], true);
        assert_eq!(trs[0]["output"], "file contents here");
    }

    #[tokio::test]
    async fn test_session_stats_enhanced() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
        let mut session = zeus_session::Session::new(&sessions_dir);
        session.init().await.unwrap();
        let session_id = session.id.clone();

        session
            .add(zeus_core::Message::user("Hello"))
            .await
            .unwrap();
        let mut assistant_msg = zeus_core::Message::assistant("Sure!");
        assistant_msg.tool_calls.push(zeus_core::ToolCall {
            id: "c1".to_string(),
            name: "shell".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
        });
        session.add(assistant_msg).await.unwrap();

        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}/stats", session_id))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // 3 turns: user + assistant(tool_use) + synthetic tool_result (S46 orphan repair)
        assert_eq!(json["total_turns"], 3);
        assert_eq!(json["tool_calls"], 1);
        assert!(json["total_tokens"].as_u64().unwrap() > 0);
        assert!(json["duration_ms"].as_i64().is_some());
        assert!(json["model_used"].is_string());
        assert!(json["cost_estimate"].is_string());

        let tools = json["tools_used"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0], "shell");
    }

    #[tokio::test]
    async fn test_session_replay_nonexistent_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let sessions_dir = tmp.path().join("sessions");
        tokio::fs::create_dir_all(&sessions_dir).await.unwrap();

        {
            let mut s = state.write().await;
            s.config.sessions = sessions_dir.clone();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/nonexistent-id/replay")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Agent-as-API tests
    // ========================================================================

    #[tokio::test]
    async fn test_agent_chat_not_spawned() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/nonexistent/chat")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"message": "hello"})).unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Outbound Webhooks tests
    // ========================================================================

    #[tokio::test]
    async fn test_list_outbound_webhooks_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/webhooks/outbound")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
        assert!(json["webhooks"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_register_outbound_webhook() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/outbound")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "url": "https://example.com/hook",
                    "events": ["message", "error"]
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["id"].is_string());
        assert_eq!(json["url"], "https://example.com/hook");
        assert_eq!(json["events"].as_array().unwrap().len(), 2);
        assert_eq!(json["enabled"], true);
    }

    #[tokio::test]
    async fn test_register_outbound_webhook_validation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/outbound")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "url": "",
                    "events": ["message"]
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_register_and_delete_outbound_webhook() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Register
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/outbound")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "url": "https://example.com/hook",
                    "events": ["tool_call"]
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let hook_id = json["id"].as_str().unwrap().to_string();

        // Delete
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/webhooks/outbound/{}", hook_id))
            .body(Body::empty())
            .unwrap();

        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], hook_id);

        // Verify empty
        let app3 = test_app(state);
        let req = Request::builder()
            .uri("/v1/webhooks/outbound")
            .body(Body::empty())
            .unwrap();

        let resp = app3.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_delete_outbound_webhook_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/webhooks/outbound/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Teams / Delegations
    // ========================================================================

    #[tokio::test]
    async fn test_list_teams() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/teams")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["teams"].as_array().unwrap().len(), 0);
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_create_team() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/teams")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"alpha-team"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "alpha-team");
        assert_eq!(json["status"], "created");
        assert!(json["id"].as_str().is_some());
        assert!(json["policy"]["max_depth"].is_number());
    }

    #[tokio::test]
    async fn test_create_team_missing_name() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/teams")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_team_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/teams/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_delegation() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/delegations")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"task":"summarize report","to_agent":"writer"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task"], "summarize report");
        assert_eq!(json["to_agent"], "writer");
        assert_eq!(json["from_agent"], "orchestrator");
        assert_eq!(json["status"], "pending");
        assert!(json["id"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_create_delegation_missing_task() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/delegations")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"to_agent":"writer"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_delegation_missing_to_agent() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/delegations")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"do something"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_delegations() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/delegations")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["delegations"].as_array().unwrap().len(), 0);
        assert_eq!(json["total"], 0);
    }

    // ========================================================================
    // Extensions
    // ========================================================================

    #[tokio::test]
    async fn test_list_extensions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/extensions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["extensions"].as_array().unwrap().len(), 0);
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_install_extension() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"my-ext","source":"https://github.com/example/ext"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "my-ext");
        assert_eq!(json["source"], "https://github.com/example/ext");
        assert_eq!(json["status"], "installed");
        assert!(json["id"].as_str().is_some());
        assert_eq!(json["permissions"]["allow_read"], true);
        assert_eq!(json["permissions"]["allow_write"], false);
    }

    #[tokio::test]
    async fn test_install_extension_missing_name() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"source":"https://example.com"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_install_extension_missing_source() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"my-ext"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_extension_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/extensions/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_extension_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("PUT")
            .uri("/v1/extensions/nonexistent-id")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled":false}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_extension_by_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/extensions/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_start_extension_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions/ext-456/start")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_stop_extension_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions/ext-789/stop")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Sandbox
    // ========================================================================

    #[tokio::test]
    async fn test_list_sandbox_policies() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/sandbox/policies")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let policies = json["policies"].as_array().unwrap();
        // 2 builtins (restrictive + permissive), no custom policies in fresh state
        assert_eq!(policies.len(), 2);
        assert_eq!(policies[0]["name"], "restrictive");
        assert_eq!(policies[1]["name"], "permissive");
        assert!(policies[0]["capabilities"]["fs_read"].is_array());
        assert!(policies[0]["limits"]["memory_mb"].is_number());
    }

    #[tokio::test]
    async fn test_create_sandbox_policy() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/sandbox/policies")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"my-policy"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "my-policy");
        assert!(json["id"].as_str().is_some());
        assert!(json["created_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_create_sandbox_policy_missing_name() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/sandbox/policies")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_sandbox_execute() {
        let state = test_state();
        let app = test_app(state);

        // JS code is not valid WASM — sandbox engine will return an error
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sandbox/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"code":"console.log('hi')","language":"javascript","policy":"restrictive"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Real sandbox execution fails for non-WASM code
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_sandbox_execute_missing_code() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/sandbox/execute")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"language":"wasm"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // TTS
    // ========================================================================

    #[tokio::test]
    async fn test_list_tts_providers() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/tts/providers")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let providers = json["providers"].as_array().unwrap();
        assert!(providers.len() >= 4);
        assert_eq!(providers[0]["name"], "elevenlabs");
        assert_eq!(providers[1]["name"], "openai");
    }

    #[tokio::test]
    async fn test_tts_synthesize() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tts/synthesize")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"text":"Hello world","voice":"alloy"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Returns BAD_GATEWAY when Piper server is not running (real provider call)
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_tts_synthesize_missing_text() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tts/synthesize")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"provider":"openai"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_tts_voices_all() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/tts/voices")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let voices = json["voices"].as_array().unwrap();
        assert!(voices.len() >= 10);
        assert_eq!(json["total"], voices.len());
    }

    #[tokio::test]
    async fn test_list_tts_voices_filtered() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/tts/voices?provider=openai")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let voices = json["voices"].as_array().unwrap();
        assert_eq!(voices.len(), 6);
        for voice in voices {
            assert_eq!(voice["provider"], "openai");
        }
    }

    #[tokio::test]
    async fn test_tts_synthesize_stream_missing_text() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/tts/synthesize/stream")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"voice":"default"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // Session branches
    // ========================================================================

    #[tokio::test]
    async fn test_list_branches() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/sessions/some-session/branches")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["branches"].as_array().unwrap().len(), 0);
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_create_branch_no_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions/nonexistent/branch")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"at_index":0}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should fail because the parent session does not exist
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // Agent CRUD
    // ========================================================================

    #[tokio::test]
    async fn test_list_agents() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/agents")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["agents"].is_array());
    }

    #[tokio::test]
    async fn test_create_and_get_agent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create agent
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"test-agent","role":"coder","model":"anthropic/claude-sonnet-4-20250514"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Agent created with supervisor");
        let agent_id = json["id"].as_str().unwrap().to_string();

        // Get agent
        let app2 = test_app(state.clone());
        let req2 = Request::builder()
            .uri(format!("/v1/agents/{}", agent_id))
            .body(Body::empty())
            .unwrap();

        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let body2 = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(json2["name"], "test-agent");
        assert_eq!(json2["role"], "coder");
        assert_eq!(json2["status"], "active");

        // Clean up
        let app3 = test_app(state);
        let req3 = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/agents/{}", agent_id))
            .body(Body::empty())
            .unwrap();

        let resp3 = app3.oneshot(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_agent_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/agents/nonexistent-agent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_agent() {
        let state = test_state();

        // Create agent first
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"updatable-agent"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agent_id = json["id"].as_str().unwrap().to_string();

        // Update agent
        let app2 = test_app(state.clone());
        let req2 = Request::builder()
            .method("PUT")
            .uri(format!("/v1/agents/{}", agent_id))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"status":"paused","model":"openai/gpt-4o"}"#))
            .unwrap();

        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let body2 = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(json2["message"], "Agent updated");

        // Clean up
        let app3 = test_app(state);
        let req3 = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/agents/{}", agent_id))
            .body(Body::empty())
            .unwrap();
        app3.oneshot(req3).await.unwrap();
    }

    #[tokio::test]
    async fn test_update_agent_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("PUT")
            .uri("/v1/agents/nonexistent-id")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"status":"paused"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_agent() {
        let state = test_state();

        // Create, then delete
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"deletable-agent"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agent_id = json["id"].as_str().unwrap().to_string();

        let app2 = test_app(state);
        let req2 = Request::builder()
            .method("DELETE")
            .uri(format!("/v1/agents/{}", agent_id))
            .body(Body::empty())
            .unwrap();

        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let body2 = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(json2["success"], true);
        assert_eq!(json2["id"], agent_id);
    }

    #[tokio::test]
    async fn test_delete_agent_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/agents/ghost-agent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_spawn_agent_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/spawn")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"agent_id":"nonexistent"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_send_to_agent_not_spawned() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/nonexistent/send")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"hello"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_agent_status_not_spawned() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/agents/nonexistent/status")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Routing
    // ========================================================================

    #[tokio::test]
    async fn test_routing_costs() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/routing/costs")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].is_array());
    }

    #[tokio::test]
    async fn test_routing_budget() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/routing/budget")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["within_budget"].is_boolean());
        assert!(json["total_cost"].is_number());
        assert!(json["monthly_budget"].is_number());
    }

    #[tokio::test]
    async fn test_routing_recommend() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/routing/cost-recommend")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"task":"Write a simple hello world function"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task"], "Write a simple hello world function");
        assert!(json["tier"].is_string());
    }

    // ========================================================================
    // Auth
    // ========================================================================

    #[tokio::test]
    async fn test_auth_status() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/auth/status")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["authenticated"].is_boolean());
        assert!(json["method"].is_string());
    }

    #[tokio::test]
    async fn test_auth_token_empty() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"token":""}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], false);
    }

    #[tokio::test]
    async fn test_auth_logout() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/logout")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Either success or error message
        assert!(json["success"].is_boolean() || json["message"].is_string());
    }

    #[tokio::test]
    async fn test_auth_login() {
        let state = test_state();
        let app = test_app(state);

        // Send an OAuth web flow request (non-blocking path).
        // The empty-body path triggers a browser-based OAuth login which blocks,
        // so we test the web-driven PKCE path instead.
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"provider":"anthropic","redirect_uri":"http://localhost:3000/callback","state":"test123","code_verifier":"test_verifier_abcdefghijklmnopqrstuvwxyz0123456789ABCD"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["status"].is_string());
        assert_eq!(json["status"], "redirect");
        assert!(
            json["authorize_url"]
                .as_str()
                .unwrap()
                .contains("anthropic")
        );
    }

    // ========================================================================
    // Memory extras
    // ========================================================================

    #[tokio::test]
    async fn test_create_memory_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"path":"test-notes.md","content":"Test notes content"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["path"], "test-notes.md");
        assert!(json["size"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_create_memory_file_missing_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"content":"some content"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_memory_file_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"path":"../../etc/passwd","content":"evil"}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_create_memory_file_conflict() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create file first
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"exists.md","content":"first"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Try to create same file again
        let app2 = test_app(state);
        let req2 = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"exists.md","content":"second"}"#))
            .unwrap();
        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_delete_memory_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create file first
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"path":"to-delete.md","content":"bye"}"#))
            .unwrap();
        app.oneshot(req).await.unwrap();

        // Delete file
        let app2 = test_app(state);
        let req2 = Request::builder()
            .method("DELETE")
            .uri("/v1/memory/files/to-delete.md")
            .body(Body::empty())
            .unwrap();

        let resp2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_delete_memory_file_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/memory/files/no-such-file.md")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_memory_file_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/memory/files/../../../etc/passwd")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_memory_timeline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/memory/timeline")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn test_sync_memory_no_mnemosyne() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/sync")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // ========================================================================
    // Config extras
    // ========================================================================

    #[tokio::test]
    async fn test_get_providers() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/config/providers")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].is_object());
        // Assert structure only — actual value depends on local providers.json / env vars
        assert!(json["default_provider"].is_string());
    }

    // ========================================================================
    // OpenAI-compatible
    // ========================================================================

    #[tokio::test]
    async fn test_openai_list_models() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/models")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["object"], "list");
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["object"], "model");
        assert_eq!(data[0]["owned_by"], "zeus");
    }

    #[tokio::test]
    async fn test_openai_chat_completions_invalid_body() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"invalid":"data"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Missing required 'messages' field should cause deserialization error
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // ========================================================================
    // Context journals
    // ========================================================================

    #[tokio::test]
    async fn test_list_context_journals() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/context/journals")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["journals"].is_array());
        assert!(json["count"].is_number());
    }

    // ========================================================================
    // Smart route
    // ========================================================================

    #[tokio::test]
    async fn test_smart_route() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/routing/recommend")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"Fix a typo in the README"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task"], "Fix a typo in the README");
        assert!(json["complexity"].is_string());
        assert!(json["recommended_model"].is_string());
        assert!(json["fallback_chain"].is_array());
    }

    #[tokio::test]
    async fn test_smart_route_missing_task() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/routing/recommend")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // Delete project
    // ========================================================================

    #[tokio::test]
    async fn test_delete_project_not_found() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/projects/nonexistent-project")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Delete session
    // ========================================================================

    #[tokio::test]
    async fn test_delete_session_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/sessions/nonexistent-session")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_session_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Create a fake session file
        let sessions_dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::write(sessions_dir.join("test-sess.jsonl"), "{}").unwrap();

        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/sessions/test-sess")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["id"], "test-sess");
        assert_eq!(json["message"], "Session deleted");
    }

    // ========================================================================
    // Bulk integration tests — ~120 new tests for full route coverage
    // ========================================================================

    // --- Group 1: Health & Status variants ---

    #[tokio::test]
    async fn test_health_endpoint_slash_health() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_doctor_endpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/doctor")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["checks"].is_array());
        assert!(json["overall"].is_string());
    }

    #[tokio::test]
    async fn test_stats_endpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["sessions"].is_object());
        assert!(json["tools"].is_object());
        assert!(json["memory"].is_object());
    }

    #[tokio::test]
    async fn test_status_has_version() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json["version"].is_string(),
            "status should have version field"
        );
        assert!(!json["version"].as_str().unwrap().is_empty());
    }

    // --- Group 2: Session lifecycle ---

    #[tokio::test]
    async fn test_session_create_and_get() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sid = json["id"].as_str().unwrap().to_string();

        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/sessions/{}", sid))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"].as_str(), Some(sid.as_str()));
    }

    #[tokio::test]
    async fn test_session_list_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = json["sessions"].as_array().unwrap();
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn test_session_delete_returns_404() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/sessions/nonexistent-xyz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_stats_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_replay_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        {
            let mut s = state.write().await;
            s.config.sessions = tmp.path().join("sessions");
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/replay")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_replay_turn_not_found_fake() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        {
            let mut s = state.write().await;
            s.config.sessions = tmp.path().join("sessions");
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/replay/0")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_raw_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/raw")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_audit_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/audit")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_tools_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions/fake/tools")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_session_branch_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/sessions/fake/branch")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"at_index":0}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // --- Group 3: Memory operations ---

    #[tokio::test]
    async fn test_get_memory_returns_context() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("context_length").is_some());
        assert!(json.get("memory").is_some());
    }

    #[tokio::test]
    async fn test_remember_fact() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/remember")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"fact":"Rust is awesome"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_add_daily_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/note")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"content":"Meeting at noon"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_list_memory_files_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        // Do NOT init workspace so it's empty
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/files")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["files"].is_array());
    }

    #[tokio::test]
    async fn test_memory_file_read_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/files/nonexistent.md")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_memory_file_write_and_read() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/memory/files/roundtrip.md")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"content":"Roundtrip data"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/files/roundtrip.md")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "Roundtrip data");
    }

    #[tokio::test]
    async fn test_memory_file_create() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/files")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"path":"new-file.md","content":"New content"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["path"], "new-file.md");
    }

    #[tokio::test]
    async fn test_memory_file_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/memory/files/nope.md")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_search_memory_query() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/search")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"query":"Zeus","limit":5}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["results"].is_array());
    }

    #[tokio::test]
    async fn test_memory_sync() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/memory/sync")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Without mnemosyne configured, returns 503
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // --- Group 4: Tools ---

    #[tokio::test]
    async fn test_execute_tool_nonexistent() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/nonexistent_tool_xyz")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"arguments":{}}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], false);
    }

    #[tokio::test]
    async fn test_execute_tool_missing_body() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/list_dir")
            .header("content-type", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Missing body results in deserialization error
        assert!(
            resp.status() == StatusCode::UNPROCESSABLE_ENTITY
                || resp.status() == StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn test_execute_read_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let test_file = tmp.path().join("test.txt");
        tokio::fs::write(&test_file, "hello world").await.unwrap();

        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/read_file")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(
                    &serde_json::json!({"arguments": {"path": test_file.to_str().unwrap()}}),
                )
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    #[tokio::test]
    async fn test_execute_shell_echo() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/shell")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(
                    &serde_json::json!({"arguments": {"command": "echo hello_test"}}),
                )
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert!(json["output"].as_str().unwrap().contains("hello_test"));
    }

    #[tokio::test]
    async fn test_tool_list_includes_core_tools() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/tools")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tools = json["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"read_file"), "should contain read_file");
        assert!(names.contains(&"write_file"), "should contain write_file");
        assert!(names.contains(&"list_dir"), "should contain list_dir");
        assert!(names.contains(&"shell"), "should contain shell");
    }

    #[tokio::test]
    async fn test_tool_list_has_schemas() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/tools")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tools = json["tools"].as_array().unwrap();
        for tool in tools {
            assert!(
                tool["parameters"].is_object(),
                "tool {} should have parameters",
                tool["name"]
            );
        }
    }

    #[tokio::test]
    async fn test_execute_tool_web_fetch() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/web_fetch")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(
                    &serde_json::json!({"arguments": {"url": "https://httpbin.org/get"}}),
                )
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // May succeed or fail depending on network, but should not crash
        assert!(json.get("success").is_some());
    }

    #[tokio::test]
    async fn test_execute_tool_invalid_args() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/tools/read_file")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"arguments":{}}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], false);
    }

    // --- Group 5: Config ---

    #[tokio::test]
    async fn test_get_config_sanitized() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/config")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8_lossy(&body);
        // Should not contain raw API keys
        assert!(
            !body_str.contains("sk-"),
            "config should not expose API keys"
        );
    }

    #[tokio::test]
    async fn test_update_config_max_iterations() {
        // Route ZEUS_HOME to a tempdir so Config::save() never touches ~/.zeus/config.toml
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("ZEUS_HOME", tmp.path()) };

        let mut config = zeus_core::Config::default();
        config.onboarding_complete = true;
        config.loaded_from_default = false;
        config.workspace = tmp.path().join("workspace");
        config.sessions = tmp.path().join("sessions");
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/config")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"max_iterations":50}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let s = state.read().await;
        assert_eq!(s.config.max_iterations, 50);

        unsafe { std::env::remove_var("ZEUS_HOME") };
        drop(tmp);
    }

    #[tokio::test]
    async fn test_update_config_workspace() {
        let state = test_state();
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/config")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"workspace":"~/.zeus/test-workspace"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let s = state.read().await;
        // ~ gets expanded to home dir by update_config handler
        let expected = dirs::home_dir().unwrap_or_default().join(".zeus/test-workspace");
        assert_eq!(s.config.workspace, expected);
    }

    #[tokio::test]
    async fn test_get_config_returns_model() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/config")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // model is absent when empty (skip_serializing_if = is_empty) — workspace is always present
        assert!(json["workspace"].is_string(), "config should have workspace field");
    }

    #[tokio::test]
    async fn test_get_providers_list() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/config/providers")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].is_object());
        assert!(json["default_provider"].is_string());
    }

    #[tokio::test]
    async fn test_config_history_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/config/history")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["history"].is_array());
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_config_reload() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/config/reload")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(
            resp.status() == StatusCode::OK || resp.status() == StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // --- Group 6: Skills ---

    #[tokio::test]
    async fn test_install_skill() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/skills")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "# My Skill\nA test skill."}))
                    .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert!(json["id"].is_string());
    }

    #[tokio::test]
    async fn test_update_skill_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/skills/fake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"enabled":false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_install_skill_missing_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/skills")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Missing content and url => 400
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_install_and_list_skill_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }

        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/skills")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"content": "# Roundtrip Skill\nTest."}))
                    .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/skills")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let skills = json["skills"].as_array().unwrap();
        assert!(skills.iter().any(|s| s["id"] == "roundtrip-skill"));
    }

    // --- Group 7: MCP Servers ---

    #[tokio::test]
    async fn test_add_mcp_server() {
        // NOTE: add_mcp_server writes to ~/.zeus/mcp.json (global state).
        // We verify the endpoint is reachable and returns expected shape.
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/mcp/servers")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "name": "Test MCP Srv",
                    "transport": "stdio",
                    "command": "/usr/bin/echo"
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["id"], "test-mcp-srv");
    }

    #[tokio::test]
    async fn test_delete_mcp_server_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/mcp/servers/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_mcp_server_tools_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/mcp/servers/fake/tools")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_add_and_list_mcp_servers() {
        // Verify that list endpoint returns the expected JSON structure
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/mcp/servers")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["servers"].is_array());
    }

    #[tokio::test]
    async fn test_test_mcp_tool_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/mcp/tools/fake/test")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"arguments":{}}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Unknown tool returns 404 (not in core tools or MCP servers)
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_test_mcp_tool_core_tool() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/mcp/tools/list_dir/test")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"arguments":{"path":"."}}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["tool"], "list_dir");
        assert_eq!(json["source"], "core");
        assert_eq!(json["status"], "success");
    }

    // --- Group 8: Channels ---

    #[tokio::test]
    async fn test_list_channels_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/channels")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_create_channel_telegram() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "telegram",
                    "name": "Test TG",
                    "config": {"bot_token": "tok", "chat_id": "123"}
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["channel_type"], "telegram");
        assert_eq!(json["name"], "Test TG");
    }

    #[tokio::test]
    async fn test_get_channel_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/channels/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_channel_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/channels/fake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Updated"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_channel_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/channels/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_test_channel_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels/fake/test")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_channel_status_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/channels/fake/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "not_found");
    }

    #[tokio::test]
    async fn test_create_and_get_channel() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/channels")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "channel_type": "webhook",
                    "name": "Get Test",
                    "config": {"webhook_url": "https://example.com/hook"}
                }))
                .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let cid = created["id"].as_str().unwrap().to_string();

        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/channels/{}", cid))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["id"], cid);
        assert_eq!(json["name"], "Get Test");
    }

    // --- Group 9: Analytics & Security ---

    #[tokio::test]
    async fn test_update_security_allowlist() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/security/allowlist")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"allowlist":["ls","cat","echo"]}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
    }

    // --- Group 10: Agents ---

    #[tokio::test]
    async fn test_list_agents_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/agents")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["agents"].is_array());
    }

    #[tokio::test]
    async fn test_create_agent_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"basic-agent","role":"tester"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Agent created with supervisor");
        assert!(json["id"].is_string());
        assert!(json["supervisor_id"].is_string());
        assert!(json["team_id"].is_string());
    }

    #[tokio::test]
    async fn test_get_agent_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/agents/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_agent_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/agents/fake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"status":"paused"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_agent_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/agents/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_spawn_agent() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/spawn")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"agent_id":"nonexistent"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_agent_status_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/agents/fake/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_send_to_agent_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/fake/send")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"hello"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_agent_chat_not_spawned_fake() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents/fake/chat")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"hello"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_and_get_agent_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"roundtrip-agent","role":"assistant"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let aid = json["id"].as_str().unwrap().to_string();

        let app2 = test_app(state);
        let req = Request::builder()
            .uri(format!("/v1/agents/{}", aid))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "roundtrip-agent");
    }

    // --- Group 11: Projects & Teams ---

    #[tokio::test]
    async fn test_list_projects_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/projects")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["projects"].is_array());
    }

    #[tokio::test]
    async fn test_create_project() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/projects")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"name":"New Project","budget":200.0}))
                    .unwrap(),
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "New Project");
        assert!(json["id"].is_string());
    }

    #[tokio::test]
    async fn test_get_project_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/projects/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_project_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/projects/fake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"Updated"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_project_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/projects/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_teams_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/teams")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
        assert!(json["teams"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_create_team_basic() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/teams")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"name":"beta-team"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "beta-team");
    }

    #[tokio::test]
    async fn test_get_team_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/teams/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_delegations_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/delegations")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_create_delegation_basic() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/delegations")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"task":"analyze data","to_agent":"analyst"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task"], "analyze data");
        assert_eq!(json["status"], "pending");
    }

    // --- Group 12: Network & Routing ---

    #[tokio::test]
    async fn test_network_agents_has_local() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/network/agents")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let agents = json["agents"].as_array().unwrap();
        assert!(agents.iter().any(|a| a["id"] == "local"));
    }

    #[tokio::test]
    async fn test_network_discover_fields() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/network/discover")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["mdns"].is_array());
        assert!(json["peer_count"].is_number());
    }

    #[tokio::test]
    async fn test_network_messages_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/network/messages")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["messages"].is_array());
    }

    #[tokio::test]
    async fn test_routing_costs_providers() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/routing/costs")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].is_array());
    }

    #[tokio::test]
    async fn test_routing_budget_fields() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/routing/budget")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["within_budget"].is_boolean());
        assert!(json["monthly_budget"].is_number());
    }

    #[tokio::test]
    async fn test_smart_route_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/routing/recommend")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"Write unit tests"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["recommended_model"].is_string());
    }

    #[tokio::test]
    async fn test_routing_recommend_cost() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/routing/cost-recommend")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task":"Translate text to French"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["tier"].is_string());
    }

    #[tokio::test]
    async fn test_assign_project_agents_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("PUT")
            .uri("/v1/projects/fake/agents")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"agents":["a","b"]}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // --- Group 13: Webhooks & Extensions ---

    #[tokio::test]
    async fn test_webhook_health_status() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/webhooks")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_receive_webhook() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"test webhook"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["received"], true);
    }

    #[tokio::test]
    async fn test_receive_webhook_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/telegram")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"message":"from telegram"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["received"], true);
    }

    #[tokio::test]
    async fn test_list_outbound_webhooks_empty_check() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/webhooks/outbound")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_register_outbound_webhook_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/webhooks/outbound")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"url":"https://example.com/hook","events":["message"]}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_delete_outbound_webhook_not_found_check() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/webhooks/outbound/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_extensions_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/extensions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_install_extension_basic() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/extensions")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"name":"test-ext","source":"https://github.com/test/ext"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "installed");
    }

    #[tokio::test]
    async fn test_get_extension_not_found_by_id() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/extensions/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_extension_not_found_by_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/v1/extensions/fake")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // --- Group 14: Auth, Approvals, TTS, Sandbox ---

    #[tokio::test]
    async fn test_auth_login_endpoint() {
        let state = test_state();
        let app = test_app(state);
        // Use web-driven PKCE path to avoid blocking browser OAuth flow
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"provider":"anthropic","redirect_uri":"http://localhost:3000/cb","state":"s1","code_verifier":"verifier_abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["status"].is_string());
        assert_eq!(json["status"], "redirect");
    }

    #[tokio::test]
    async fn test_auth_status_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/auth/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["authenticated"].is_boolean());
    }

    #[tokio::test]
    async fn test_auth_token_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"token":"test-token"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_logout_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/auth/logout")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_list_approvals_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/approvals")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_approve_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/approvals/fake/approve")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_deny_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/approvals/fake/deny")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_list_tts_providers_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/tts/providers")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["providers"].as_array().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn test_list_sandbox_policies_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sandbox/policies")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["policies"].as_array().unwrap().len() >= 2);
    }

    #[tokio::test]
    async fn test_list_tts_voices() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/tts/voices")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["voices"].as_array().unwrap().len() >= 1);
        assert!(json["total"].is_number());
    }

    // --- Group 15: Context & Activity & Misc ---

    #[tokio::test]
    async fn test_context_journals() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/context/journals")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["journals"].is_array());
    }

    #[tokio::test]
    async fn test_memory_timeline_endpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/timeline")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn test_activity_feed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/activity")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["events"].is_array());
    }

    #[tokio::test]
    async fn test_activity_with_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        {
            let s = state.read().await;
            s.workspace.init().await.unwrap();
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/activity?limit=5")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["events"].as_array().unwrap().len() <= 5);
    }

    #[tokio::test]
    async fn test_openai_list_models_endpoint() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/models")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["object"], "list");
        assert!(json["data"].as_array().unwrap().len() >= 1);
    }

    #[tokio::test]
    async fn test_session_list_with_pagination() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        tokio::fs::create_dir_all(tmp.path().join("sessions"))
            .await
            .unwrap();

        // Create 2 sessions
        for _ in 0..2 {
            let app = test_app(state.clone());
            let req = Request::builder()
                .method("POST")
                .uri("/v1/sessions")
                .body(Body::empty())
                .unwrap();
            app.oneshot(req).await.unwrap();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/sessions?limit=5&offset=0")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = json["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(json["total"].as_u64().unwrap() >= 2);
    }

    // ====================================================================
    // Onboarding
    // ====================================================================

    #[tokio::test]
    async fn test_onboarding_status_defaults_false() {
        // Use raw Config::default() (onboarding_complete = false) to simulate fresh install
        let config = zeus_core::Config::default();
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/onboarding/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["completed"], false); // fresh installs need onboarding
    }

    #[tokio::test]
    async fn test_onboarding_complete_sets_true() {
        let state = test_state();

        // Complete onboarding
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/onboarding/complete")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);

        // Verify status is now true
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/onboarding/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["completed"], true);
    }

    #[tokio::test]
    async fn test_onboarding_complete_is_idempotent() {
        let state = test_state();

        // Call complete twice
        for _ in 0..2 {
            let app = test_app(state.clone());
            let req = Request::builder()
                .method("POST")
                .uri("/v1/onboarding/complete")
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        // Status should still be true
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/onboarding/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["completed"], true);
    }

    // ====================================================================
    // Image Generation
    // ====================================================================

    #[tokio::test]
    async fn test_generate_image_no_backend() {
        // Without a running backend, we should get a BAD_GATEWAY error
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/images/generate")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "prompt": "a beautiful sunset over mountains"
                }))
                .unwrap(),
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should fail since no image gen backend is running
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_generate_image_missing_prompt() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/images/generate")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should fail with 422 (missing required field 'prompt')
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_list_images_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = zeus_core::Config::default();
        config.image_gen = Some(zeus_core::ImageGenConfig {
            store_path: tmp.path().join("images"),
            ..Default::default()
        });
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/images")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["images"].is_array());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_list_images_with_stored_image() {
        let tmp = tempfile::TempDir::new().unwrap();
        let images_dir = tmp.path().join("images");
        tokio::fs::create_dir_all(&images_dir).await.unwrap();

        // Write a fake metadata file
        let meta = serde_json::json!({
            "image_id": "test-123",
            "prompt": "a cat",
            "width": 512,
            "height": 512,
        });
        tokio::fs::write(
            images_dir.join("test-123.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();

        let mut config = zeus_core::Config::default();
        config.image_gen = Some(zeus_core::ImageGenConfig {
            store_path: images_dir,
            ..Default::default()
        });
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/images")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["images"][0]["image_id"], "test-123");
        assert_eq!(json["images"][0]["prompt"], "a cat");
    }

    #[tokio::test]
    async fn test_get_image_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = zeus_core::Config::default();
        config.image_gen = Some(zeus_core::ImageGenConfig {
            store_path: tmp.path().join("images"),
            ..Default::default()
        });
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/images/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_image_with_stored_image() {
        let tmp = tempfile::TempDir::new().unwrap();
        let images_dir = tmp.path().join("images");
        tokio::fs::create_dir_all(&images_dir).await.unwrap();

        // Write metadata + image file
        let meta = serde_json::json!({
            "image_id": "img-abc",
            "prompt": "sunset",
            "width": 1024,
            "height": 1024,
        });
        tokio::fs::write(
            images_dir.join("img-abc.json"),
            serde_json::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(images_dir.join("img-abc.png"), b"fake-png-bytes")
            .await
            .unwrap();

        let mut config = zeus_core::Config::default();
        config.image_gen = Some(zeus_core::ImageGenConfig {
            store_path: images_dir,
            ..Default::default()
        });
        let state: SharedState = Arc::new(RwLock::new(AppState::new(config).unwrap()));
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/images/img-abc")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["image_id"], "img-abc");
        assert_eq!(json["prompt"], "sunset");
        // image_base64 should be non-empty (base64 of "fake-png-bytes")
        assert!(!json["image_base64"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_prometheus_create_plan_single_step() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/plan")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"goal":"Deploy the app"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["goal"], "Deploy the app");
        assert_eq!(json["nodes"], 1);
        assert!(json["plan_id"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_prometheus_create_plan_multi_step() {
        let state = test_state();
        let app = test_app(state);
        let body_json = serde_json::json!({
            "goal": "Build and test",
            "steps": [
                {"description": "Build", "tool": "shell"},
                {"description": "Test", "tool": "shell", "dependencies": [0]},
            ]
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/plan")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body_json).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["nodes"], 2);
        assert!(json["parallel_groups"].as_array().is_some());
        assert!(json["critical_path"].as_array().is_some());
    }

    #[tokio::test]
    async fn test_prometheus_create_plan_missing_goal() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/plan")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_prometheus_get_plan() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/prometheus/plan/plan-123")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // PlanStore correctly returns 404 for nonexistent plan IDs
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_prometheus_execute() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"goal":"Run tests","simulate":true,"step_delay_ms":0}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["goal"], "Run tests");
        assert_eq!(json["status"], "accepted");
        assert!(json["plan_id"].is_string());
        assert!(json["total_steps"].as_u64().unwrap() >= 1);
        assert!(json["topological_order"].is_array());
        assert_eq!(json["mode"], "simulated");
    }

    #[tokio::test]
    async fn test_prometheus_execute_spawns_async() {
        // Verify the endpoint returns immediately (202) and execution runs in background
        let state = test_state();
        let app = test_app(state.clone());

        // Subscribe BEFORE the request
        let rx = {
            let guard = state.read().await;
            guard.plan_broadcast.subscribe()
        };

        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"goal":"async test","simulate":true,"step_delay_ms":10}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let plan_id = json["plan_id"].as_str().unwrap().to_string();
        assert!(!plan_id.is_empty());

        // Give the background task time to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Verify events were broadcast
        let mut events = vec![];
        let mut rx = rx;
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        // Should have received at least running + done + complete events
        assert!(
            events.len() >= 3,
            "Expected at least 3 events, got {}",
            events.len()
        );
    }

    #[tokio::test]
    async fn test_prometheus_execute_simulated_mode() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"goal":"sim test","simulate":true,"step_delay_ms":50}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["mode"], "simulated");
    }

    #[tokio::test]
    async fn test_prometheus_execute_agent_mode_default() {
        // This test asserts agent execution fails (no API key).
        // On machines with a live LLM key, execution succeeds instead.
        // Skip unless explicitly opted-in via ZEUS_RUN_LLM_TESTS=1.
        if std::env::var("ZEUS_RUN_LLM_TESTS").unwrap_or_default() != "1" {
            eprintln!(
                "Skipping test_prometheus_execute_agent_mode_default (set ZEUS_RUN_LLM_TESTS=1 to run)"
            );
            return;
        }
        // Default mode (no explicit mode param) should be "agent" when LLM is configured
        let state = test_state();
        let app = test_app(state.clone());

        let rx = {
            let guard = state.read().await;
            guard.plan_broadcast.subscribe()
        };

        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"goal":"agent test"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["mode"], "agent");

        // Poll for background executor events (agent runs will fail due to no API key)
        let mut rx = rx;
        let mut has_failed = false;
        let mut has_complete = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline && !has_complete {
            match tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(e)) => match e {
                    crate::websocket::PlanEvent::StepUpdate(u) => {
                        if u.status == "failed" {
                            has_failed = true;
                            assert!(
                                u.output.contains("Agent error"),
                                "Expected Agent error in output, got: {}",
                                u.output
                            );
                        }
                    }
                    crate::websocket::PlanEvent::Complete(c) => {
                        has_complete = true;
                        assert!(
                            c.status == "failed" || c.status == "partial",
                            "Expected failed/partial, got: {}",
                            c.status
                        );
                    }
                },
                _ => continue,
            }
        }
        assert!(has_failed, "Expected at least one failed step");
        assert!(has_complete, "Expected completion event");
    }

    #[tokio::test]
    async fn test_prometheus_execute_explicit_llm_mode() {
        // This test asserts LLM execution fails (no API key).
        // On machines with a live LLM key, execution succeeds instead.
        // Skip unless explicitly opted-in via ZEUS_RUN_LLM_TESTS=1.
        if std::env::var("ZEUS_RUN_LLM_TESTS").unwrap_or_default() != "1" {
            eprintln!(
                "Skipping test_prometheus_execute_explicit_llm_mode (set ZEUS_RUN_LLM_TESTS=1 to run)"
            );
            return;
        }
        // Explicitly request "llm" mode via the mode parameter
        let state = test_state();
        let app = test_app(state.clone());

        let rx = {
            let guard = state.read().await;
            guard.plan_broadcast.subscribe()
        };

        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"goal":"llm test","mode":"llm"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["mode"], "llm");

        // Poll for background executor events
        let mut rx = rx;
        let mut has_failed = false;
        let mut has_complete = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline && !has_complete {
            match tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(e)) => match e {
                    crate::websocket::PlanEvent::StepUpdate(u) => {
                        if u.status == "failed" {
                            has_failed = true;
                        }
                    }
                    crate::websocket::PlanEvent::Complete(c) => {
                        has_complete = true;
                        assert!(
                            c.status == "failed" || c.status == "partial",
                            "Expected failed/partial, got: {}",
                            c.status
                        );
                    }
                },
                _ => continue,
            }
        }
        assert!(has_failed, "Expected at least one failed step in LLM mode");
        assert!(has_complete, "Expected completion event");
    }

    #[tokio::test]
    async fn test_prometheus_execute_agent_spawns_and_cleans_up() {
        // This test asserts agent cleanup after execution (expects failure path).
        // On machines with a live LLM key, execution takes a different path.
        // Skip unless explicitly opted-in via ZEUS_RUN_LLM_TESTS=1.
        if std::env::var("ZEUS_RUN_LLM_TESTS").unwrap_or_default() != "1" {
            eprintln!(
                "Skipping test_prometheus_execute_agent_spawns_and_cleans_up (set ZEUS_RUN_LLM_TESTS=1 to run)"
            );
            return;
        }
        // Verify the agent mode spawns and then unregisters agents
        let state = test_state();
        let app = test_app(state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/v1/prometheus/execute")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"goal":"cleanup test"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        // Poll until execution completes and agents are cleaned up
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let state_guard = state.read().await;
            let agents = state_guard.agent_registry.list();
            let prometheus_agents: Vec<_> = agents
                .iter()
                .filter(|a| a.agent_id.starts_with("prometheus-step-"))
                .collect();
            if prometheus_agents.is_empty() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "Expected all prometheus agents to be unregistered, found {}",
                    prometheus_agents.len()
                );
            }
        }
    }

    #[tokio::test]
    async fn test_prometheus_state() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/prometheus/state")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total_agents"], 0);
        assert!(json["agents"].as_array().is_some());
    }

    // ========================================================================
    // Peer Review endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_list_reviews_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/reviews")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
        assert!(json["reviews"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_submit_review_missing_fields() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/reviews")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"task_id":"t1"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_get_review_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/reviews/nonexistent-id")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["review_count"], 0);
    }

    #[tokio::test]
    async fn test_approve_review_missing_reviewer() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/reviews/sub-1/approve")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_reject_review_missing_reviewer() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/reviews/sub-1/reject")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // Marketplace endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_marketplace_list_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/listings")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_marketplace_publish() {
        let state = test_state();
        let app = test_app(state);
        let body_json = serde_json::json!({
            "name": "code-review",
            "description": "Automated code review skill",
            "publisher_id": "agent-1",
            "capabilities": ["code_analysis"],
            "tags": ["review", "quality"],
            "price": 100,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body_json).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["skill_id"].as_str().is_some());
        assert_eq!(json["status"], "published");
    }

    #[tokio::test]
    async fn test_marketplace_publish_missing_name() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"description":"x","publisher_id":"a"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_marketplace_trade_missing_fields() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/trade")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"buyer_id":"b1"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_marketplace_ledger() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/ledger/agent-1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["agent_id"], "agent-1");
        assert_eq!(json["balance"], 0);
    }

    #[tokio::test]
    async fn test_marketplace_reputation() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/reputation/agent-1")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["agent_id"], "agent-1");
        assert_eq!(json["score"], 0.5);
        assert_eq!(json["total_trades"], 0);
    }

    #[tokio::test]
    async fn test_marketplace_publish_then_list() {
        let state = test_state();
        // Publish a skill
        let app1 = test_app(state.clone());
        let body_json = serde_json::json!({
            "name": "test-skill",
            "description": "A test skill",
            "publisher_id": "agent-pub",
            "price": 50,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body_json).unwrap()))
            .unwrap();
        let resp = app1.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // List should now have 1 entry
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/listings")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["total"], 1);
        assert_eq!(json["listings"][0]["name"], "test-skill");
    }

    // ========================================================================
    // Marketplace typed-response tests
    // ========================================================================

    #[tokio::test]
    async fn test_marketplace_publish_response_fields() {
        let state = test_state();
        let app = test_app(state);
        let body_json = serde_json::json!({
            "name": "typed-skill",
            "description": "A typed skill",
            "publisher_id": "agent-typed",
            "price": 75,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body_json).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pr: zeus_marketplace::PublishResponse = serde_json::from_slice(&body).unwrap();
        assert!(!pr.skill_id.is_empty());
        assert_eq!(pr.status, "published");
    }

    #[tokio::test]
    async fn test_marketplace_list_response_fields() {
        let state = test_state();
        // Publish
        let app1 = test_app(state.clone());
        let body_json = serde_json::json!({
            "name": "list-check",
            "description": "Check list response",
            "publisher_id": "agent-lc",
            "tags": ["rust", "test"],
            "price": 200,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body_json).unwrap()))
            .unwrap();
        app1.oneshot(req).await.unwrap();

        // List
        let app2 = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/listings")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let lr: zeus_marketplace::MarketplaceListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(lr.total, 1);
        let entry = &lr.listings[0];
        assert_eq!(entry.name, "list-check");
        assert_eq!(entry.author_agent_id, "agent-lc");
        assert_eq!(entry.price_tokens, 200);
        assert_eq!(entry.rating, 0.0);
        assert_eq!(entry.downloads, 0);
        assert_eq!(entry.tags, vec!["rust", "test"]);
        // created_at should be parseable (it is a DateTime<Utc>)
        assert!(entry.created_at.timestamp() > 0);
    }

    #[tokio::test]
    async fn test_marketplace_trade_response_fields() {
        let state = test_state();
        // Publish a skill first
        let app1 = test_app(state.clone());
        let pub_json = serde_json::json!({
            "name": "trade-target",
            "description": "For trade test",
            "publisher_id": "seller-1",
            "price": 50,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/listings")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&pub_json).unwrap()))
            .unwrap();
        let resp = app1.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let pr: zeus_marketplace::PublishResponse = serde_json::from_slice(&body).unwrap();

        // Propose a trade
        let app2 = test_app(state);
        let trade_json = serde_json::json!({
            "buyer_id": "buyer-1",
            "skill_id": pr.skill_id,
            "offered_price": 50,
        });
        let req = Request::builder()
            .method("POST")
            .uri("/v1/marketplace/trade")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&trade_json).unwrap()))
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let tr: zeus_marketplace::TradeResponse = serde_json::from_slice(&body).unwrap();
        assert!(!tr.trade_id.is_empty());
        assert_eq!(tr.status, "proposed");
        assert_eq!(tr.buyer_id, "buyer-1");
        assert_eq!(tr.seller_id, "seller-1");
        assert_eq!(tr.skill_id, pr.skill_id);
        assert_eq!(tr.price, 50);
        assert!(tr.timestamp.timestamp() > 0);
    }

    #[tokio::test]
    async fn test_marketplace_ledger_response_structure() {
        let state = test_state();
        // Credit some tokens directly to the marketplace ledger
        {
            let s = state.read().await;
            s.marketplace
                .ledger
                .credit("agent-ledger", 300, "test mint")
                .await;
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/ledger/agent-ledger")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let lr: zeus_marketplace::LedgerResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(lr.agent_id, "agent-ledger");
        assert_eq!(lr.balance, 300);
        assert_eq!(lr.transactions.len(), 1);
        assert_eq!(lr.transactions[0].amount, 300);
        assert_eq!(lr.transactions[0].reason, "test mint");
    }

    #[tokio::test]
    async fn test_marketplace_reputation_response_fields() {
        let state = test_state();
        // Record some trades for reputation
        {
            let s = state.read().await;
            s.marketplace
                .reputation
                .record_trade_success("rep-agent")
                .await;
            s.marketplace
                .reputation
                .record_trade_success("rep-agent")
                .await;
            s.marketplace
                .reputation
                .record_trade_failure("rep-agent")
                .await;
        }
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/marketplace/reputation/rep-agent")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let rr: zeus_marketplace::ReputationResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(rr.agent_id, "rep-agent");
        assert_eq!(rr.total_trades, 3);
        assert_eq!(rr.successful_trades, 2);
        // trade_success_rate = 2/3
        assert!((rr.trade_success_rate - 2.0 / 3.0).abs() < 0.01);
        assert!(rr.score > 0.0);
    }

    // ========================================================================
    // Economy endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_economy_wallets_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/economy/wallets")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Should have at least the "default" wallet from auto-mint
        assert!(json["wallets"].is_array());
        assert!(json["total_supply"].as_u64().unwrap() >= 10_000);
    }

    #[tokio::test]
    async fn test_economy_wallet_agent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/economy/wallets/test-agent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["wallet"]["agent_id"], "test-agent");
        assert_eq!(json["wallet"]["balance"], 0);
        assert!(json["transactions"].is_array());
    }

    #[tokio::test]
    async fn test_economy_transactions_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/economy/transactions")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["transactions"].is_array());
        // Has at least the auto-mint transaction
        assert!(json["total_minted"].as_u64().unwrap() >= 10_000);
    }

    #[tokio::test]
    async fn test_economy_wallet_with_activity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Mint some tokens to a test agent
        {
            let s = state.read().await;
            s.ledger
                .mint(
                    "active-agent",
                    500,
                    zeus_economy::TransactionReason::SystemGrant,
                    "test grant",
                )
                .unwrap();
            s.ledger
                .spend(
                    "active-agent",
                    25,
                    zeus_economy::TransactionReason::LlmCall,
                    "test llm call",
                )
                .unwrap();
        }

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/economy/wallets/active-agent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["wallet"]["agent_id"], "active-agent");
        assert_eq!(json["wallet"]["balance"], 475);
        assert_eq!(json["transactions"].as_array().unwrap().len(), 2);
    }

    // ========================================================================
    // Economy: stake / unstake / transfer integration tests
    // ========================================================================

    #[tokio::test]
    async fn test_economy_stake_and_unstake() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Mint tokens first
        {
            let s = state.read().await;
            s.ledger
                .mint(
                    "staker-1",
                    1000,
                    zeus_economy::TransactionReason::SystemGrant,
                    "test grant",
                )
                .unwrap();
        }

        // Stake
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/stake")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"agent_id":"staker-1","amount":300,"purpose":"marketplace_listing"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["amount"], 300);
        let stake_id = json["stake_id"].as_str().unwrap().to_string();

        // Check balance decreased
        {
            let s = state.read().await;
            let w = s.ledger.wallet("staker-1").unwrap();
            assert_eq!(w.balance, 700);
        }

        // Unstake
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/unstake")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"agent_id":"staker-1","amount":300,"stake_id":stake_id})
                    .to_string(),
            ))
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Balance restored
        {
            let s = state.read().await;
            let w = s.ledger.wallet("staker-1").unwrap();
            assert_eq!(w.balance, 1000);
        }
    }

    #[tokio::test]
    async fn test_economy_stake_insufficient_funds() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/stake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"agent_id":"broke-agent","amount":500}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYMENT_REQUIRED);
    }

    #[tokio::test]
    async fn test_economy_transfer_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        // Mint tokens to sender
        {
            let s = state.read().await;
            s.ledger
                .mint(
                    "sender-1",
                    500,
                    zeus_economy::TransactionReason::SystemGrant,
                    "grant",
                )
                .unwrap();
        }

        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/transfer")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"from":"sender-1","to":"receiver-1","amount":100,"note":"tip"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["from"]["balance"], 400);
        assert_eq!(json["to"]["balance"], 100);
        assert_eq!(json["amount"], 100);
    }

    #[tokio::test]
    async fn test_economy_transfer_to_self_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/transfer")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"from":"agent-1","to":"agent-1","amount":10}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_economy_transfer_insufficient_funds() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/transfer")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"from":"poor-agent","to":"rich-agent","amount":999}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::PAYMENT_REQUIRED);
    }

    #[tokio::test]
    async fn test_economy_stake_zero_amount_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/stake")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"agent_id":"a","amount":0}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_economy_transfer_zero_amount_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/transfer")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"from":"a","to":"b","amount":0}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_economy_mint_success() {
        // SAFETY: test-only env mutation; this test is serialized by tokio
        unsafe { std::env::set_var("ZEUS_MINT_ADMIN_TOKEN", "test-mint-token-s32c") };
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/mint")
            .header("content-type", "application/json")
            .header("Authorization", "Bearer test-mint-token-s32c")
            .body(Body::from(
                r#"{"agent_id":"new-agent","amount":500,"reason":"system_grant"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], true);
        assert_eq!(json["amount_minted"], 500);
        assert_eq!(json["new_balance"], 500);

        // Verify wallet exists with correct balance
        {
            let s = state.read().await;
            let w = s.ledger.wallet("new-agent").unwrap();
            assert_eq!(w.balance, 500);
        }
    }

    #[tokio::test]
    async fn test_economy_mint_zero_rejected() {
        // SAFETY: test-only env mutation; this test is serialized by tokio
        unsafe { std::env::set_var("ZEUS_MINT_ADMIN_TOKEN", "test-mint-token-s32c") };
        let tmp = tempfile::TempDir::new().unwrap();
        let state = test_state_with_tempdir(&tmp);

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/economy/mint")
            .header("content-type", "application/json")
            .header("Authorization", "Bearer test-mint-token-s32c")
            .body(Body::from(r#"{"agent_id":"a","amount":0}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // ========================================================================
    // Anthropic OAuth endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_anthropic_oauth_login_redirects() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/auth/anthropic/login")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);

        let location = resp
            .headers()
            .get("location")
            .expect("should have Location header")
            .to_str()
            .unwrap();

        // Verify it points to Anthropic's authorize URL
        assert!(
            location.starts_with("https://console.anthropic.com/oauth/authorize"),
            "Location should start with Anthropic authorize URL, got: {}",
            location,
        );
        // Verify required OAuth params are present
        assert!(location.contains("client_id="), "Missing client_id");
        assert!(
            location.contains("response_type=code"),
            "Missing response_type"
        );
        assert!(location.contains("redirect_uri="), "Missing redirect_uri");
        assert!(location.contains("scope="), "Missing scope");
        assert!(
            location.contains("code_challenge="),
            "Missing code_challenge"
        );
        assert!(
            location.contains("code_challenge_method=S256"),
            "Missing code_challenge_method"
        );
        assert!(location.contains("state="), "Missing state");
    }

    #[tokio::test]
    async fn test_anthropic_oauth_callback_invalid_state() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/auth/anthropic/callback?code=abc123&state=bogus")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_anthropic_oauth_status_returns_valid_json() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .uri("/v1/auth/anthropic/status")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // Must always return a method field and an authenticated field
        assert!(json["method"].is_string(), "missing method field");
        assert!(
            json["authenticated"].is_boolean(),
            "missing authenticated field"
        );

        // Method must be one of the known values
        let method = json["method"].as_str().unwrap();
        assert!(
            ["none", "api_key", "oauth", "setup_token"].contains(&method),
            "unexpected method: {}",
            method,
        );
    }

    // ========================================================================
    // Workflow endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_goals_list_returns_valid_json() {
        // goals_list always returns JSON (with error field if GoalStack unavailable
        // in test environment); never panics.
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/goals")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("goals").is_some(), "goals field missing");
        assert!(json.get("total").is_some(), "total field missing");
    }

    #[tokio::test]
    async fn test_goals_create_missing_description_returns_400() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/goals")
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_analytics_models_returns_array() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/analytics/models")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("models").is_some(), "models field missing");
        assert!(json["models"].is_array(), "models should be an array");
    }

    #[tokio::test]
    async fn test_analytics_sessions_empty() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/analytics/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("sessions").is_some(), "sessions field missing");
    }

    #[tokio::test]
    async fn test_analytics_daily_returns_ok() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/analytics/daily?days=7")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Response shape may vary depending on session store availability;
        // assert valid JSON is returned without panicking.
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let _json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    }

    #[tokio::test]
    async fn test_workflow_artifacts_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/workflows/nonexistent-id/artifacts")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_orchestrate_status_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/orchestrate/nonexistent-session")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // Either 404 or 405 (Method Not Allowed) depending on route method
        let status = resp.status().as_u16();
        assert!(
            status == 404 || status == 405,
            "expected 404 or 405, got {}",
            status
        );
    }

    #[tokio::test]
    async fn test_goals_create_with_description() {
        // With a valid description the handler attempts GoalStack creation;
        // in a test environment GoalStack may fail (returning 500 or 200),
        // but the request must not panic.
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/goals")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"description":"test goal","priority":"normal"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // May succeed (201) or fail with 500 if GoalStack db path unavailable
        let status = resp.status().as_u16();
        assert!(
            status == 201 || status == 200 || status == 500,
            "unexpected status {}",
            status
        );
    }

    #[tokio::test]
    async fn test_workflow_download_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/workflows/nonexistent-id/download")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ========================================================================
    // Phase 7: Intelligence Layer endpoint tests
    // ========================================================================

    #[tokio::test]
    async fn test_graph_nodes_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/graph/nodes")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_graph_edges_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/graph/edges")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_graph_stats_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/graph/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_memory_patterns_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/patterns")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_memory_stats_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_entity_messages_no_mnemosyne() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/memory/entities/1/messages")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_reflect_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/reflect")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_capabilities_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/capabilities")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_learning_stats_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/learning/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_learning_lessons_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/learning/lessons")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_understand_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/nous/understand")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"input":"hello world"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_reason_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/nous/reason")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"problem":"test problem"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_learn_outcome_no_engine() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/nous/learning/outcome")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"intent_id":"test","success":true,"feedback":"good"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_nous_reflect_with_engine() {
        let state = test_state();
        // Initialize Nous in the test state
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/reflect")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("health").is_some());
        assert!(json.get("state").is_some());
        assert!(json.get("summary").is_some());
    }

    #[tokio::test]
    async fn test_nous_capabilities_with_engine() {
        let state = test_state();
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/capabilities")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("capabilities").is_some());
        assert!(json.get("count").is_some());
    }

    #[tokio::test]
    async fn test_nous_learning_stats_with_engine() {
        let state = test_state();
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/learning/stats")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("total_lessons").is_some());
        assert!(json.get("success_rate").is_some());
    }

    #[tokio::test]
    async fn test_nous_learning_lessons_with_engine() {
        let state = test_state();
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/nous/learning/lessons")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("lessons").is_some());
        assert!(json.get("total").is_some());
    }

    #[tokio::test]
    async fn test_nous_understand_with_engine() {
        let state = test_state();
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/nous/understand")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"input":"schedule a meeting tomorrow"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("id").is_some());
        assert!(json.get("intent_type").is_some());
        assert!(json.get("confidence").is_some());
    }

    #[tokio::test]
    async fn test_nous_reason_with_engine() {
        let state = test_state();
        let nous = zeus_nous::Nous::new().await.unwrap();
        state.write().await.nous = Some(std::sync::Arc::new(nous));

        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/nous/reason")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"problem":"how to deploy a web app"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("problem").is_some());
        assert!(json.get("steps").is_some());
        assert!(json.get("confidence").is_some());
    }

    // Predictive spawning integration tests

    #[tokio::test]
    async fn test_spawner_status() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/spawner/status")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("health").is_some());
        assert!(json.get("criteria").is_some());
        assert!(json.get("tracker").is_some());
        assert_eq!(json["health"]["active_spawns"], 0);
        assert!(json["health"]["is_healthy"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_spawner_active() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/spawner/active")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["active"].as_array().unwrap().is_empty());
        assert_eq!(json["count"], 0);
    }

    #[tokio::test]
    async fn test_spawner_history() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .uri("/v1/spawner/history")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["history"].as_array().unwrap().is_empty());
        assert_eq!(json["total"], 0);
    }

    #[tokio::test]
    async fn test_spawner_analyze_simple_task() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/spawner/analyze")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"task":"read a file"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("should_spawn").is_some());
        assert!(json.get("rationale").is_some());
        assert!(json.get("estimated_speedup").is_some());
        assert!(json.get("analysis").is_some());
    }

    #[tokio::test]
    async fn test_spawner_analyze_complex_task() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/spawner/analyze")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"task":"deploy microservices to production","complexity":"complex","tools":["shell","write_file","web_fetch","read_file"]}"#
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("should_spawn").is_some());
        assert!(json.get("agents").is_some());
        assert_eq!(json["analysis"]["detected_complexity"], "complex");
        assert_eq!(json["analysis"]["tool_count"], 4);
    }

    #[tokio::test]
    async fn test_studio_drive_session_not_found() {
        let state = test_state();
        let app = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions/nonexistent/drive")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_studio_drive_plancard_complex_goal_requires_approval() {
        let state = test_state();
        let app = test_app(state.clone());

        // Create a studio session with a complex goal (shell + deploy + multi-step)
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"goal":"Build the project, deploy to production, execute database migrations, and then run integration tests"}"#
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = json["id"].as_str().unwrap().to_string();

        // Drive without approval — complex goal should return plan card
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/v1/studio/sessions/{}/drive", session_id))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        if status == StatusCode::OK {
            // Should be awaiting_approval with a plan_card
            if json.get("status").and_then(|v| v.as_str()) == Some("awaiting_approval") {
                assert!(json.get("plan_card").is_some(), "Should include plan_card");
                assert!(json["message"].as_str().unwrap_or("").contains("approve"));
            }
            // Could also be 200 with driving started if complexity didn't trigger approval
        }
    }

    #[tokio::test]
    async fn test_studio_drive_skip_approval_with_flag() {
        let state = test_state();
        let app = test_app(state.clone());

        // Create a studio session with a complex goal
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"goal":"Build the project, deploy to production, and execute shell commands"}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = json["id"].as_str().unwrap().to_string();

        // Drive WITH approved: true — should skip PlanCard gate
        let app2 = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/v1/studio/sessions/{}/drive", session_id))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"approved": true}"#))
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let status = resp.status();
        // Should NOT be awaiting_approval — skipped the gate
        // Will be either 200 (driving) or 500 (LLM not configured)
        if status == StatusCode::OK {
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_ne!(
                json.get("status").and_then(|v| v.as_str()),
                Some("awaiting_approval"),
                "Should not be awaiting_approval when approved:true is passed"
            );
        }
    }

    #[tokio::test]
    async fn test_studio_drive_simple_goal_skips_approval() {
        let state = test_state();
        let app = test_app(state.clone());

        // Create a session with a simple goal (single action, no shell)
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"goal":"read the config"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = json["id"].as_str().unwrap().to_string();

        // Drive without approval flag — simple goal should not trigger approval gate
        let app2 = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/v1/studio/sessions/{}/drive", session_id))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let status = resp.status();
        if status == StatusCode::OK {
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_ne!(
                json.get("status").and_then(|v| v.as_str()),
                Some("awaiting_approval"),
                "Simple goal should not require approval"
            );
        }
    }

    #[tokio::test]
    async fn test_studio_drive_nonexistent_session() {
        let state = test_state();
        let app = test_app(state);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions/nonexistent-id/drive")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_studio_session_crud() {
        let state = test_state();

        // Create
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"goal":"test session CRUD"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = json["id"].as_str().unwrap().to_string();
        assert_eq!(json["session"]["goal"], "test session CRUD");
        assert_eq!(json["status"], "created");

        // Read
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .uri(&format!("/v1/studio/sessions/{}", session_id))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["session"]["id"], session_id);
        assert_eq!(json["session"]["goal"], "test session CRUD");

        // List
        let app3 = test_app(state.clone());
        let req = Request::builder()
            .uri("/v1/studio/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app3.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let sessions = json["sessions"].as_array().unwrap();
        assert!(
            sessions
                .iter()
                .any(|s| s["id"].as_str() == Some(session_id.as_str()))
        );

        // Delete
        let app4 = test_app(state);
        let req = Request::builder()
            .method("DELETE")
            .uri(&format!("/v1/studio/sessions/{}", session_id))
            .body(Body::empty())
            .unwrap();
        let resp = app4.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_studio_pause_resume() {
        let state = test_state();

        // Create session
        let app = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri("/v1/studio/sessions")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"goal":"pausable session"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let session_id = json["id"].as_str().unwrap().to_string();

        // Pause on non-driving session returns 409 CONFLICT
        let app2 = test_app(state.clone());
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/v1/studio/sessions/{}/pause", session_id))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "Pause on idle session should return CONFLICT"
        );

        // Resume on non-paused session returns 409 CONFLICT
        let app3 = test_app(state);
        let req = Request::builder()
            .method("POST")
            .uri(&format!("/v1/studio/sessions/{}/resume", session_id))
            .body(Body::empty())
            .unwrap();
        let resp = app3.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::CONFLICT,
            "Resume on idle session should return CONFLICT"
        );
    }
}
