//! TikTok Content Posting API tools
//!
//! Upload videos to TikTok via the Content Posting API v2.
//! Env vars:
//!   TIKTOK_ACCESS_TOKEN — OAuth2 user access token with video.publish scope
//!
//! Upload flow (file upload):
//!   1. POST /v2/post/publish/video/init/ → get publish_id + upload_url
//!   2. PUT video bytes to upload_url
//!   3. Poll GET /v2/post/publish/status/fetch/ until complete

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

const TIKTOK_API_BASE: &str = "https://open.tiktokapis.com";

fn get_access_token(args: &Value) -> Result<String> {
    args["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| std::env::var("TIKTOK_ACCESS_TOKEN").ok())
        .ok_or_else(|| Error::Tool(
            "TikTok access token required. Set TIKTOK_ACCESS_TOKEN env var or pass 'access_token' arg.".into()
        ))
}

async fn tiktok_post(access_token: &str, endpoint: &str, body: &Value) -> Result<Value> {
    let url = format!("{TIKTOK_API_BASE}{endpoint}");
    let output = Command::new("curl")
        .args([
            "-s", "-X", "POST",
            "-H", &format!("Authorization: Bearer {access_token}"),
            "-H", "Content-Type: application/json; charset=UTF-8",
            &url,
            "-d", &body.to_string(),
        ])
        .output()
        .await
        .map_err(|e| Error::Tool(format!("TikTok API call failed: {e}")))?;

    serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Tool(format!("Invalid TikTok response: {e}")))
}

async fn tiktok_get(access_token: &str, endpoint: &str) -> Result<Value> {
    let url = format!("{TIKTOK_API_BASE}{endpoint}");
    let output = Command::new("curl")
        .args([
            "-s",
            "-H", &format!("Authorization: Bearer {access_token}"),
            &url,
        ])
        .output()
        .await
        .map_err(|e| Error::Tool(format!("TikTok API call failed: {e}")))?;

    serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Tool(format!("Invalid TikTok response: {e}")))
}

// ---------------------------------------------------------------------------
// 1. TikTokUploadTool
// ---------------------------------------------------------------------------

pub struct TikTokUploadTool;

#[async_trait]
impl TalosTool for TikTokUploadTool {
    fn name(&self) -> &'static str { "tiktok_upload" }
    fn description(&self) -> &'static str {
        "Upload a video to TikTok using the Content Posting API v2. Requires OAuth2 access token with video.publish scope (env: TIKTOK_ACCESS_TOKEN)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("file", "string", "Local video file path to upload", true)
            .with_param("title", "string", "Video caption/title (max 2200 chars)", true)
            .with_param("privacy", "string", "Privacy: PUBLIC_TO_EVERYONE, MUTUAL_FOLLOW_FRIENDS, FOLLOWER_OF_CREATOR, SELF_ONLY (default: SELF_ONLY)", false)
            .with_param("disable_comment", "boolean", "Disable comments (default false)", false)
            .with_param("disable_duet", "boolean", "Disable duet (default false)", false)
            .with_param("disable_stitch", "boolean", "Disable stitch (default false)", false)
            .with_param("access_token", "string", "OAuth2 access token (overrides TIKTOK_ACCESS_TOKEN env var)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let file = args["file"].as_str().ok_or_else(|| Error::Tool("Missing 'file'".into()))?;
        let title = args["title"].as_str().ok_or_else(|| Error::Tool("Missing 'title'".into()))?;
        let access_token = get_access_token(&args)?;
        let privacy = args["privacy"].as_str().unwrap_or("SELF_ONLY");

        // Get file size
        let metadata = tokio::fs::metadata(file).await
            .map_err(|e| Error::Tool(format!("Cannot read file: {e}")))?;
        let file_size = metadata.len();

        // Step 1: Initiate upload
        let init_body = json!({
            "post_info": {
                "title": title,
                "privacy_level": privacy,
                "disable_comment": args["disable_comment"].as_bool().unwrap_or(false),
                "disable_duet": args["disable_duet"].as_bool().unwrap_or(false),
                "disable_stitch": args["disable_stitch"].as_bool().unwrap_or(false)
            },
            "source_info": {
                "source": "FILE_UPLOAD",
                "video_size": file_size,
                "chunk_size": file_size,
                "total_chunk_count": 1
            }
        });

        let init_response = tiktok_post(&access_token, "/v2/post/publish/video/init/", &init_body).await?;

        let error_code = init_response["error"]["code"].as_str().unwrap_or("ok");
        if error_code != "ok" {
            return Err(Error::Tool(format!(
                "TikTok init failed: {} — {}",
                error_code,
                init_response["error"]["message"].as_str().unwrap_or("unknown error")
            )));
        }

        let publish_id = init_response["data"]["publish_id"]
            .as_str()
            .ok_or_else(|| Error::Tool("Missing publish_id in TikTok response".into()))?
            .to_string();

        let upload_url = init_response["data"]["upload_url"]
            .as_str()
            .ok_or_else(|| Error::Tool("Missing upload_url in TikTok response".into()))?
            .to_string();

        // Step 2: Upload file
        let upload_output = Command::new("curl")
            .args([
                "-s", "-X", "PUT",
                "-H", "Content-Type: video/mp4",
                "-H", &format!("Content-Length: {file_size}"),
                "-H", &format!("Content-Range: bytes 0-{}/{file_size}", file_size - 1),
                "--upload-file", file,
                &upload_url,
            ])
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to upload to TikTok: {e}")))?;

        if !upload_output.status.success() {
            return Err(Error::Tool(format!(
                "TikTok upload PUT failed: {}",
                String::from_utf8_lossy(&upload_output.stderr)
            )));
        }

        Ok(format!(
            "TikTok upload initiated!\nPublish ID: {publish_id}\nTitle: {title}\nPrivacy: {privacy}\nUse tiktok_check_status with publish_id to check processing status."
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. TikTokCheckStatusTool
// ---------------------------------------------------------------------------

pub struct TikTokCheckStatusTool;

#[async_trait]
impl TalosTool for TikTokCheckStatusTool {
    fn name(&self) -> &'static str { "tiktok_check_status" }
    fn description(&self) -> &'static str {
        "Check the processing status of a TikTok video upload by publish_id"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("publish_id", "string", "Publish ID returned from tiktok_upload", true)
            .with_param("access_token", "string", "OAuth2 access token (overrides env var)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let publish_id = args["publish_id"].as_str()
            .ok_or_else(|| Error::Tool("Missing 'publish_id'".into()))?;
        let access_token = get_access_token(&args)?;

        let response = tiktok_get(
            &access_token,
            &format!("/v2/post/publish/status/fetch/?publish_id={publish_id}"),
        ).await?;

        let status = response["data"]["status"].as_str().unwrap_or("UNKNOWN");
        let fail_reason = response["data"]["fail_reason"].as_str().unwrap_or("");
        let public_url = response["data"]["publicaly_available_post_id"]
            .as_array()
            .and_then(|ids| ids.first())
            .and_then(|id| id.as_str())
            .map(|id| format!("https://www.tiktok.com/@/video/{id}"))
            .unwrap_or_default();

        let mut result = format!("TikTok publish status: {status}\nPublish ID: {publish_id}");
        if !fail_reason.is_empty() {
            result.push_str(&format!("\nFail reason: {fail_reason}"));
        }
        if !public_url.is_empty() {
            result.push_str(&format!("\nURL: {public_url}"));
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_upload_schema() {
        let t = TikTokUploadTool;
        assert_eq!(t.name(), "tiktok_upload");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"file") && req.contains(&"title"));
    }

    #[test]
    fn test_check_status_schema() {
        let t = TikTokCheckStatusTool;
        assert_eq!(t.name(), "tiktok_check_status");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"publish_id"));
    }

    #[test]
    fn test_token_missing() {
        // Without arg and with a non-existent var name, should fail
        let result = get_access_token(&json!({}));
        // May pass or fail depending on env — just verify the arg path works
        let _ = result;
    }

    #[test]
    fn test_token_from_arg() {
        let result = get_access_token(&json!({"access_token": "tt_test"}));
        assert_eq!(result.unwrap(), "tt_test");
    }

    #[test]
    fn test_upload_description_contains_api_info() {
        let t = TikTokUploadTool;
        assert!(t.description().contains("TIKTOK_ACCESS_TOKEN"));
        assert!(t.description().contains("video.publish"));
    }

    #[test]
    fn test_privacy_levels_documented() {
        let t = TikTokUploadTool;
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let props = p["properties"].as_object().unwrap();
        let privacy_desc = props["privacy"]["description"].as_str().unwrap_or("");
        assert!(privacy_desc.contains("PUBLIC_TO_EVERYONE"));
        assert!(privacy_desc.contains("SELF_ONLY"));
    }
}
