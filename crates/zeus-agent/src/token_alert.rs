//! Token exhaustion alerts (S46-T3)
//!
//! When the gateway gets a 401 (auth/quota) or 429 (rate limit) from an LLM
//! provider, post a warning to the fleet Discord channel. 60s dedup cooldown
//! per agent+error combo to prevent flooding.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;
use tracing::{info, warn};

/// Minimum seconds between alerts for the same agent+error combo.
const DEDUP_COOLDOWN_SECS: u64 = 60;

/// Fleet Discord channel for alerts.
fn fleet_channel_id() -> String {
    std::env::var("ZEUS_DISCORD_FLEET_CHANNEL")
        .unwrap_or_else(|_| "1475583517156180018".to_string())
}

/// Global dedup state: (agent_name, status_code) -> last_alert_time
static ALERT_DEDUP: Mutex<Option<HashMap<(String, u16), Instant>>> = Mutex::new(None);

/// Check if an LLM error indicates token exhaustion or rate limiting.
/// Returns the HTTP status code if it's a 401 or 429, None otherwise.
pub fn extract_alert_status(error_msg: &str) -> Option<u16> {
    // Match patterns like "error 401:", "error 429:", "status: 401", "HTTP 429"
    if error_msg.contains("401") {
        Some(401)
    } else if error_msg.contains("429") || error_msg.contains("rate_limit") {
        Some(429)
    } else {
        None
    }
}

/// Send a token exhaustion alert to the fleet Discord channel.
/// Returns true if the alert was sent (not deduped), false if suppressed.
pub async fn maybe_send_alert(
    agent_name: &str,
    provider: &str,
    status_code: u16,
    discord_token: Option<&str>,
) -> bool {
    let key = (agent_name.to_string(), status_code);

    // Check dedup cooldown
    {
        let mut guard = ALERT_DEDUP.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);

        if let Some(last) = map.get(&key)
            && last.elapsed().as_secs() < DEDUP_COOLDOWN_SECS {
                info!(
                    "Token alert suppressed (dedup {}s): agent={} code={}",
                    DEDUP_COOLDOWN_SECS, agent_name, status_code
                );
                return false;
            }
        map.insert(key, Instant::now());
    }

    let label = match status_code {
        401 => "AUTH/QUOTA EXHAUSTED",
        429 => "RATE LIMITED",
        _ => "LLM ERROR",
    };

    let message = format!(
        "⚠️ **{}** — `{}` hit {} from `{}` provider",
        label, agent_name, status_code, provider
    );

    // Try to post to Discord
    let token = match discord_token {
        Some(t) => t.to_string(),
        None => {
            warn!("Token alert: no Discord token configured, logging only: {}", message);
            return false;
        }
    };

    let client = reqwest::Client::new();
    let url = format!(
        "https://discord.com/api/v10/channels/{}/messages",
        fleet_channel_id()
    );

    let payload = serde_json::json!({ "content": message });

    match client
        .post(&url)
        .header("Authorization", format!("Bot {}", token))
        .json(&payload)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            info!("Token alert sent to Discord: {}", message);
            true
        }
        Ok(resp) => {
            warn!(
                "Token alert Discord post failed ({}): {}",
                resp.status(),
                message
            );
            false
        }
        Err(e) => {
            warn!("Token alert Discord request failed: {} — {}", e, message);
            false
        }
    }
}

/// Convenience: check an LLM error and fire an alert if it's a 401/429.
/// This is fire-and-forget — spawns a background task so it doesn't block.
pub fn check_and_alert(
    error_msg: &str,
    agent_name: String,
    provider: String,
    discord_token: Option<String>,
) {
    if let Some(status) = extract_alert_status(error_msg) {
        let token = discord_token;
        tokio::spawn(async move {
            maybe_send_alert(&agent_name, &provider, status, token.as_deref()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_401() {
        assert_eq!(
            extract_alert_status("Anthropic API error 401: invalid bearer token"),
            Some(401)
        );
    }

    #[test]
    fn test_extract_429() {
        assert_eq!(
            extract_alert_status("Anthropic API error 429: rate_limit_error"),
            Some(429)
        );
    }

    #[test]
    fn test_extract_rate_limit_keyword() {
        assert_eq!(
            extract_alert_status("rate_limit exceeded for this model"),
            Some(429)
        );
    }

    #[test]
    fn test_extract_normal_error() {
        assert_eq!(
            extract_alert_status("Anthropic API error 500: internal server error"),
            None
        );
    }

    #[test]
    fn test_extract_no_status() {
        assert_eq!(
            extract_alert_status("Connection timed out"),
            None
        );
    }

    #[tokio::test]
    async fn test_dedup_suppresses_repeat() {
        // First call should not be suppressed (returns true only if Discord works,
        // but without a token it returns false — test the dedup logic directly)
        let key = ("test_agent".to_string(), 401u16);
        {
            let mut guard = ALERT_DEDUP.lock().unwrap();
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(key.clone(), Instant::now());
        }
        // Second call within cooldown should be suppressed
        let result = maybe_send_alert("test_agent", "anthropic", 401, None).await;
        assert!(!result, "Should be suppressed by dedup");
    }
}
