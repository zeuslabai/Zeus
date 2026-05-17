//! Instagram Graph API tools
//!
//! Provides tools for publishing content to Instagram Business accounts
//! via the Instagram Graph API (v21.0).
//!
//! Auth uses a long-lived User Access Token. Pass via the `access_token`
//! argument or set the `INSTAGRAM_ACCESS_TOKEN` environment variable.
//! The business account ID comes from `account_id` arg or
//! `INSTAGRAM_BUSINESS_ACCOUNT_ID` env var.
//!
//! Photo upload flow:  create container → publish
//! Reel upload flow:   create container → poll until FINISHED → publish

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

const GRAPH_API: &str = "https://graph.facebook.com/v21.0";
/// Maximum poll attempts when waiting for a video container to finish processing.
const MAX_POLL_ATTEMPTS: u32 = 30;
/// Seconds between each poll attempt.
const POLL_INTERVAL_SECS: u64 = 5;

// ---------------------------------------------------------------------------
// Credential helpers
// ---------------------------------------------------------------------------

fn get_access_token(args: &Value) -> Result<String> {
    if let Some(t) = args.get("access_token").and_then(|v| v.as_str()) {
        return Ok(t.to_string());
    }
    std::env::var("INSTAGRAM_ACCESS_TOKEN").map_err(|_| {
        Error::Tool(
            "Missing 'access_token' parameter and INSTAGRAM_ACCESS_TOKEN env var not set"
                .to_string(),
        )
    })
}

fn get_account_id(args: &Value) -> Result<String> {
    if let Some(id) = args.get("account_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    std::env::var("INSTAGRAM_BUSINESS_ACCOUNT_ID").map_err(|_| {
        Error::Tool(
            "Missing 'account_id' parameter and INSTAGRAM_BUSINESS_ACCOUNT_ID env var not set"
                .to_string(),
        )
    })
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// POST to a full Graph API URL with form-encoded body.
async fn graph_post_url(url: &str, params: &[(&str, &str)]) -> Result<Value> {
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .form(params)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Instagram API request failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Instagram API error {}: {}",
            status, text
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

/// GET a Graph API URL (query params baked into the URL string).
async fn graph_get(url: &str) -> Result<Value> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Tool(format!("Instagram API GET failed: {}", e)))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| Error::Tool(format!("Failed to read response: {}", e)))?;

    if !status.is_success() {
        return Err(Error::Tool(format!(
            "Instagram API error {}: {}",
            status, text
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        Error::Tool(format!(
            "Invalid JSON: {} (body: {})",
            e,
            &text[..zeus_core::floor_char_boundary(&text, 200)]
        ))
    })
}

// ---------------------------------------------------------------------------
// Tool: instagram_send_photo
// ---------------------------------------------------------------------------

/// Publish a photo to an Instagram Business account.
pub struct InstagramSendPhotoTool;

#[async_trait]
impl TalosTool for InstagramSendPhotoTool {
    fn name(&self) -> &'static str {
        "instagram_send_photo"
    }

    fn description(&self) -> &'static str {
        "Publish a photo to an Instagram Business account via the Graph API"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "image_url",
                "string",
                "Publicly accessible URL of the image to post",
                true,
            )
            .with_param("caption", "string", "Caption for the post", false)
            .with_param(
                "account_id",
                "string",
                "Instagram Business Account ID (falls back to INSTAGRAM_BUSINESS_ACCOUNT_ID env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Long-lived User Access Token (falls back to INSTAGRAM_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_access_token(&args)?;
        let account_id = get_account_id(&args)?;

        let image_url = args
            .get("image_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required 'image_url' parameter".to_string()))?;

        let caption = args.get("caption").and_then(|v| v.as_str()).unwrap_or("");

        // Step 1: Create media container
        let container_url = format!("{}/{}/media", GRAPH_API, account_id);
        let mut params = vec![("image_url", image_url), ("access_token", token.as_str())];
        if !caption.is_empty() {
            params.push(("caption", caption));
        }
        let container = graph_post_url(&container_url, &params).await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("No container ID in response".to_string()))?
            .to_string();

        // Step 2: Publish
        let publish_url = format!("{}/{}/media_publish", GRAPH_API, account_id);
        let publish_params = vec![
            ("creation_id", container_id.as_str()),
            ("access_token", token.as_str()),
        ];
        let result = graph_post_url(&publish_url, &publish_params).await?;
        let post_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        Ok(format!(
            "Photo published successfully. Post ID: {}",
            post_id
        ))
    }
}

// ---------------------------------------------------------------------------
// Tool: instagram_send_reel
// ---------------------------------------------------------------------------

/// Publish a Reel (short video) to an Instagram Business account.
pub struct InstagramSendReelTool;

#[async_trait]
impl TalosTool for InstagramSendReelTool {
    fn name(&self) -> &'static str {
        "instagram_send_reel"
    }

    fn description(&self) -> &'static str {
        "Publish a Reel (short video) to an Instagram Business account via the Graph API. \
         Polls for video processing completion before publishing."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "video_url",
                "string",
                "Publicly accessible URL of the video to post as a Reel",
                true,
            )
            .with_param("caption", "string", "Caption for the Reel", false)
            .with_param(
                "share_to_feed",
                "boolean",
                "Whether to also share the Reel to the main feed (default: true)",
                false,
            )
            .with_param(
                "account_id",
                "string",
                "Instagram Business Account ID (falls back to INSTAGRAM_BUSINESS_ACCOUNT_ID env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Long-lived User Access Token (falls back to INSTAGRAM_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_access_token(&args)?;
        let account_id = get_account_id(&args)?;

        let video_url = args
            .get("video_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing required 'video_url' parameter".to_string()))?;

        let caption = args.get("caption").and_then(|v| v.as_str()).unwrap_or("");

        let share_to_feed = args
            .get("share_to_feed")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let share_str = if share_to_feed { "true" } else { "false" };

        // Step 1: Create video container
        let container_url = format!("{}/{}/media", GRAPH_API, account_id);
        let mut params = vec![
            ("media_type", "REELS"),
            ("video_url", video_url),
            ("share_to_feed", share_str),
            ("access_token", token.as_str()),
        ];
        if !caption.is_empty() {
            params.push(("caption", caption));
        }
        let container = graph_post_url(&container_url, &params).await?;
        let container_id = container
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("No container ID in response".to_string()))?
            .to_string();

        // Step 2: Poll until container status is FINISHED
        let status_url = format!(
            "{}/{}?fields=status_code,status&access_token={}",
            GRAPH_API, container_id, token
        );

        let mut attempts = 0u32;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
            attempts += 1;

            let status_resp = graph_get(&status_url).await?;
            let status_code = status_resp
                .get("status_code")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");

            match status_code {
                "FINISHED" => break,
                "ERROR" | "EXPIRED" => {
                    let detail = status_resp
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    return Err(Error::Tool(format!(
                        "Video container processing failed ({}): {}",
                        status_code, detail
                    )));
                }
                _ => {
                    if attempts >= MAX_POLL_ATTEMPTS {
                        return Err(Error::Tool(format!(
                            "Timed out waiting for video processing after {} attempts (last status: {})",
                            attempts, status_code
                        )));
                    }
                    // IN_PROGRESS or other transient state — keep waiting
                }
            }
        }

        // Step 3: Publish
        let publish_url = format!("{}/{}/media_publish", GRAPH_API, account_id);
        let publish_params = vec![
            ("creation_id", container_id.as_str()),
            ("access_token", token.as_str()),
        ];
        let result = graph_post_url(&publish_url, &publish_params).await?;
        let post_id = result
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        Ok(format!("Reel published successfully. Post ID: {}", post_id))
    }
}

// ---------------------------------------------------------------------------
// Tool: instagram_get_profile
// ---------------------------------------------------------------------------

/// Fetch public profile metadata for an Instagram Business account.
pub struct InstagramGetProfileTool;

#[async_trait]
impl TalosTool for InstagramGetProfileTool {
    fn name(&self) -> &'static str {
        "instagram_get_profile"
    }

    fn description(&self) -> &'static str {
        "Fetch profile information for an Instagram Business account (name, bio, follower count, media count)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "account_id",
                "string",
                "Instagram Business Account ID (falls back to INSTAGRAM_BUSINESS_ACCOUNT_ID env var)",
                false,
            )
            .with_param(
                "access_token",
                "string",
                "Long-lived User Access Token (falls back to INSTAGRAM_ACCESS_TOKEN env var)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let token = get_access_token(&args)?;
        let account_id = get_account_id(&args)?;

        let url = format!(
            "{}/{}?fields=id,name,biography,followers_count,media_count,profile_picture_url,website&access_token={}",
            GRAPH_API, account_id, token
        );

        let profile = graph_get(&url).await?;

        let name = profile
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("N/A");
        let bio = profile
            .get("biography")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let followers = profile
            .get("followers_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let media = profile
            .get("media_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let website = profile
            .get("website")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pic_url = profile
            .get("profile_picture_url")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        Ok(format!(
            "Name: {}\nBio: {}\nFollowers: {}\nMedia count: {}\nWebsite: {}\nProfile picture: {}",
            name, bio, followers, media, website, pic_url
        ))
    }
}
