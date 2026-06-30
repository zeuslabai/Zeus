// Channel management, health, pairing, messages

use super::*;

pub async fn fetch_channels() -> Result<ChannelsResponse, String> {
    fetch_json("/v1/channels").await
}

pub async fn create_channel(req: &CreateChannelReq) -> Result<MsgResponse, String> {
    post_json("/v1/channels", req).await
}

pub async fn update_channel(id: &str, req: &UpdateChannelReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/channels/{}", id), req).await
}

pub async fn delete_channel(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/channels/{}", id)).await
}

pub async fn test_channel(id: &str) -> Result<TestChannelResponse, String> {
    post_json(&format!("/v1/channels/{}/test", id), &serde_json::json!({})).await
}

pub async fn fetch_channel_status(channel_id: &str) -> Result<ChannelStatusResponse, String> {
    fetch_json(&format!("/v1/channels/{}/status", channel_id)).await
}

pub async fn connect_channel(channel_id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/connect", channel_id), &serde_json::json!({})).await
}

pub async fn disconnect_channel(channel_id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/disconnect", channel_id), &serde_json::json!({})).await
}

pub async fn fetch_channel_health() -> Result<ChannelHealthResponse, String> {
    fetch_json("/v1/channels/health").await
}

pub async fn pair_channel(id: &str, body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/pair", id), body).await
}

pub async fn fetch_channel_pairings(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/channels/{}/pairings", id)).await
}

pub async fn verify_channel(id: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/channels/{}/verify", id), &serde_json::json!({})).await
}

pub async fn poll_channel(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/channels/{}/poll", id)).await
}
