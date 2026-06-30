//! Integration tests for inbound channel message processing

use std::sync::Arc;
use tokio::sync::RwLock;
use zeus_api::{AppState, InboundConfig};
use zeus_channels::ChannelManager;
use zeus_core::Config;

/// Helper to create test app state
fn create_test_state() -> Arc<RwLock<AppState>> {
    let config = Config::default();
    Arc::new(RwLock::new(AppState::new(config).unwrap()))
}

#[tokio::test]
async fn test_inbound_loop_starts_and_stops() {
    let state = create_test_state();
    let channel_mgr = Arc::new(ChannelManager::new(16));

    let handle =
        zeus_api::start_inbound_loop(state, channel_mgr.clone(), InboundConfig::default()).await;

    // The loop should be running
    assert!(!handle.is_finished());

    // Abort to clean up
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn test_inbound_loop_receiver_already_taken() {
    let state = create_test_state();
    let channel_mgr = Arc::new(ChannelManager::new(16));

    // Take receiver first so the loop gets None
    {
        let _rx = channel_mgr.take_receiver();
    }

    let handle = zeus_api::start_inbound_loop(state, channel_mgr, InboundConfig::default()).await;

    // Should exit quickly since receiver was already taken
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "Loop should exit quickly when receiver is already taken"
    );
}

#[tokio::test]
async fn test_inbound_loop_with_custom_config() {
    let state = create_test_state();
    let channel_mgr = Arc::new(ChannelManager::new(16));

    let config = InboundConfig {
        max_message_len: 1024,
        ..Default::default()
    };

    let handle = zeus_api::start_inbound_loop(state, channel_mgr.clone(), config).await;

    assert!(!handle.is_finished());

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn test_multiple_inbound_loops_second_gets_no_receiver() {
    let state = create_test_state();
    let channel_mgr = Arc::new(ChannelManager::new(4));

    // First loop takes the receiver
    let handle1 =
        zeus_api::start_inbound_loop(state.clone(), channel_mgr.clone(), InboundConfig::default())
            .await;
    assert!(!handle1.is_finished());

    // Second loop should get None and exit immediately
    let handle2 = zeus_api::start_inbound_loop(state, channel_mgr, InboundConfig::default()).await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle2).await;
    assert!(
        result.is_ok(),
        "Second loop should exit since receiver was already taken"
    );

    handle1.abort();
    let _ = handle1.await;
}
