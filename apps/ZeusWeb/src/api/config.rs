// Config, providers, models, onboarding

use super::*;

pub async fn fetch_status() -> Result<StatusResponse, String> {
    fetch_json("/v1/status").await
}

pub async fn fetch_stats() -> Result<StatsResponse, String> {
    fetch_json("/v1/stats").await
}

pub async fn fetch_config() -> Result<ConfigResponse, String> {
    fetch_json("/v1/config").await
}

pub async fn save_config(config: &serde_json::Value) -> Result<MsgResponse, String> {
    put_json("/v1/config", config).await
}

pub async fn fetch_config_history() -> Result<ConfigHistoryResponse, String> {
    fetch_json("/v1/config/history").await
}

pub async fn reload_config() -> Result<MsgResponse, String> {
    post_json("/v1/config/reload", &serde_json::json!({})).await
}

// Providers

pub async fn fetch_providers() -> Result<ProvidersResponse, String> {
    fetch_json("/v1/config/providers").await
}

pub async fn test_provider_connection(provider: &str, api_key: Option<&str>, url: Option<&str>) -> Result<TestResult, String> {
    let mut body = serde_json::json!({ "provider": provider });
    if let Some(k) = api_key { body["api_key"] = serde_json::Value::String(k.to_string()); }
    if let Some(u) = url { body["url"] = serde_json::Value::String(u.to_string()); }
    post_json("/v1/config/test", &body).await
}

pub async fn fetch_providers_list() -> Result<ProvidersListResponse, String> {
    fetch_json("/v1/providers").await
}

// Onboarding

pub async fn fetch_onboarding_status() -> Result<OnboardingStatus, String> {
    fetch_json("/v1/onboarding/status").await
}

pub async fn complete_onboarding(
    mode: &str,
    provider: &str,
    auth_method: &str,
    model: Option<&str>,
    url: Option<&str>,
) -> Result<MsgResponse, String> {
    let mut body = serde_json::json!({
        "mode": mode,
        "provider": provider,
        "auth_method": auth_method,
    });
    if let Some(m) = model {
        body["model"] = serde_json::Value::String(m.to_string());
    }
    if let Some(u) = url {
        body["url"] = serde_json::Value::String(u.to_string());
    }
    post_json("/v1/onboarding/complete", &body).await
}

// Doctor

pub async fn fetch_doctor() -> Result<DoctorResponse, String> {
    fetch_json("/v1/doctor").await
}

// Pipeline

pub async fn fetch_pipeline_stats() -> Result<PipelineStatsResponse, String> {
    fetch_json("/v1/pipeline/stats").await
}

// Activity

pub async fn fetch_activity() -> Result<ActivityResponse, String> {
    fetch_json("/v1/activity").await
}
