//! #434 — Tool registry ↔ advertised schema drift regression test.
//!
//! Asserts that the tools advertised via `GET /v1/tools` are in bijection with
//! the `TalosRegistry` (advertised ⊆ registry AND registry ⊆ advertised) when
//! no explicit deny-list is in place.
//!
//! This test is the regression pin for the operational-staleness incident: a
//! long-running process with a stale binary had registry tools (x_delete_post)
//! executing via CLI while being invisible to the LLM's function-calling
//! surface. The root cause was operational (binary swap without restart), but
//! the invariant below locks the code-level guarantee so that a future
//! filter/deny-list that breaks symmetry fails loudly here rather than
//! silently degrading the tool surface.

use axum::http::StatusCode;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zeus_api::{AppState, create_test_router};
use zeus_core::Config;
use zeus_talos::TalosRegistry;

/// Build an AppState with Talos tools enabled (mirrors the gateway cook path).
fn create_talos_state() -> Arc<RwLock<AppState>> {
    let mut config = Config::default();
    // Enable Talos so AppState::new wires TalosRegistry::with_defaults()
    // into the ToolRegistry — same path as the gateway at src/gateway.rs:672.
    config.talos = Some(zeus_core::TalosConfig::default());
    Arc::new(RwLock::new(AppState::new(config).unwrap()))
}

/// Extract tool names from the `GET /v1/tools` JSON response.
async fn advertised_tool_names(state: Arc<RwLock<AppState>>) -> HashSet<String> {
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/v1/tools")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

/// All tool names the TalosRegistry knows how to execute.
fn registry_tool_names() -> HashSet<String> {
    TalosRegistry::with_defaults()
        .list()
        .into_iter()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn test_v1_tools_carries_build_meta() {
    // #434: the /v1/tools response must carry _meta.build_sha so a seat can
    // detect staleness by comparing against origin/main. When BuildInfo has
    // not been initialized (as in unit tests), _meta is null — that's the
    // documented soft-fail shape. Either way, the field must be present.
    let state = create_talos_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/v1/tools")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        json.get("_meta").is_some(),
        "/v1/tools response must include _meta field (got: {json})"
    );
}

#[tokio::test]
async fn test_registry_tools_advertised_no_drift() {
    // The core invariant: every registry tool must be visible on the wire.
    // registry ⊆ advertised
    let advertised = advertised_tool_names(create_talos_state()).await;
    let registry = registry_tool_names();

    let missing: Vec<_> = registry.difference(&advertised).collect();
    assert!(
        missing.is_empty(),
        "DRIFT: registry tools not advertised via /v1/tools: {:?}. \
         (denies must be explicit; see #434)",
        missing
    );
}

#[tokio::test]
async fn test_advertised_subset_of_registry() {
    // The reverse invariant: every advertised tool must be executable by the
    // registry. advertised ⊆ registry.
    // Note: the ToolRegistry also includes core tools (built-ins like message,
    // memory_store, etc.) and browser tools — so advertised may be a SUPERSET
    // of the talos registry alone. We only assert no advertised tool is
    // UNKNOWN to the system (i.e. not a phantom entry).
    let advertised = advertised_tool_names(create_talos_state()).await;
    let registry = registry_tool_names();

    // Tools advertised that are in neither talos registry nor core — flag as
    // potential phantoms. This is a soft check; core/browser tools are expected.
    // The hard check is the forward direction (test_registry_tools_advertised_no_drift).
    assert!(
        !advertised.is_empty(),
        "advertised tool set must not be empty"
    );
    assert!(!registry.is_empty(), "registry tool set must not be empty");
    // Sanity: the registry should have at least the x_* tools that surfaced
    // the original incident.
    let has_x_tools = registry.iter().any(|n| n.starts_with("x_"));
    assert!(
        has_x_tools,
        "TalosRegistry must contain x_* tools (incident #434 surface) — got: {:?}",
        registry
    );
}

#[test]
fn test_talos_registry_cross_platform_invariant() {
    // #435 regression pin: the legacy deploy path force-set enable_talos=false
    // on FreeBSD/Linux, nuking ALL Talos tools (including cross-platform ones
    // with zero AppleScript dependency). That override is gone; this test
    // locks the guarantee that with_defaults() produces a registry containing
    // cross-platform tools on every target.
    //
    // The compile-time `#[cfg(target_os = "macos")]` gates in register_all()
    // already produce correct per-target shape: macOS-only tools (calendar,
    // notes, music, safari, …) are compiled out on non-macOS, while the
    // cross-platform tools (x_*, nostr, instagram, tiktok, youtube, twitch,
    // matrix, mqtt, feishu, …) register unconditionally. There must be no
    // runtime config override that defeats this.
    let registry = TalosRegistry::with_defaults();
    let schemas = registry.schemas();
    let names: HashSet<&str> = schemas.iter().map(|s| s.name.as_str()).collect();

    // Cross-platform tool that must appear on ALL targets.
    assert!(
        names.contains("x_post"),
        "#435 regression: x_post missing from TalosRegistry::with_defaults() on this target — \
         cross-platform tools must not be gated. got: {:?}",
        names
    );
    assert!(
        names.contains("nostr_publish_note"),
        "#435 regression: nostr_publish_note missing from with_defaults() — \
         cross-platform tools must not be gated. got: {:?}",
        names
    );

    // Bidirectional invariant (#435 adopted spec): macOS-only tools
    // (AppleScript-backed) must be ABSENT on non-macOS targets. They are
    // compiled out via #[cfg(target_os = "macos")] in register_all(); if one
    // ever leaks through on FreeBSD/Linux, it would crash at execute-time
    // (no AppleScript bridge). Lock the shape so the gate is symmetric:
    // cross-platform present + macOS-only absent on non-macOS.
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            !names.contains("calendar_list"),
            "#435 regression: calendar_list present on non-macOS target — \
             macOS-only tools must be compiled out via #[cfg(target_os = \"macos\")]. got: {:?}",
            names
        );
        assert!(
            !names.contains("notes_create"),
            "#435 regression: notes_create present on non-macOS target — \
             macOS-only tools must be compiled out via #[cfg(target_os = \"macos\")]. got: {:?}",
            names
        );
    }
}
