//! YouTube upload tools
//!
//! Upload videos to YouTube via the YouTube Data API v3 (resumable upload).
//! Env vars:
//!   YOUTUBE_API_KEY       — API key (for public data, not needed for upload)
//!   YOUTUBE_ACCESS_TOKEN  — OAuth2 access token with youtube.upload scope
//!
//! The upload flow:
//!   1. Initiate resumable upload session → get upload URL
//!   2. Upload video bytes to the upload URL
//!   3. Return video ID

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

fn get_access_token(args: &Value) -> Result<String> {
    args["access_token"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| std::env::var("YOUTUBE_ACCESS_TOKEN").ok())
        .ok_or_else(|| Error::Tool(
            "YouTube access token required. Set YOUTUBE_ACCESS_TOKEN env var or pass 'access_token' arg.".into()
        ))
}

// ---------------------------------------------------------------------------
// 1. YouTubeUploadTool
// ---------------------------------------------------------------------------

pub struct YouTubeUploadTool;

#[async_trait]
impl TalosTool for YouTubeUploadTool {
    fn name(&self) -> &'static str { "youtube_upload" }
    fn description(&self) -> &'static str {
        "Upload a video to YouTube using the YouTube Data API v3. Requires OAuth2 access token with youtube.upload scope (env: YOUTUBE_ACCESS_TOKEN)."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("file", "string", "Local video file path to upload", true)
            .with_param("title", "string", "Video title", true)
            .with_param("description", "string", "Video description", false)
            .with_param("tags", "array", "List of tag strings", false)
            .with_param("category_id", "string", "YouTube category ID (default '22' = People & Blogs)", false)
            .with_param("privacy", "string", "Privacy status: public, unlisted, private (default: private)", false)
            .with_param("access_token", "string", "OAuth2 access token (overrides YOUTUBE_ACCESS_TOKEN env var)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let file = args["file"].as_str().ok_or_else(|| Error::Tool("Missing 'file'".into()))?;
        let title = args["title"].as_str().ok_or_else(|| Error::Tool("Missing 'title'".into()))?;
        let access_token = get_access_token(&args)?;

        let description = args["description"].as_str().unwrap_or("");
        let category_id = args["category_id"].as_str().unwrap_or("22");
        let privacy = args["privacy"].as_str().unwrap_or("private");

        let tags: Vec<String> = args["tags"].as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        // Build metadata
        let metadata = json!({
            "snippet": {
                "title": title,
                "description": description,
                "tags": tags,
                "categoryId": category_id
            },
            "status": {
                "privacyStatus": privacy
            }
        });

        // Step 1: Initiate resumable upload
        let init_output = Command::new("curl")
            .args([
                "-s", "-X", "POST",
                "-H", &format!("Authorization: Bearer {access_token}"),
                "-H", "Content-Type: application/json; charset=UTF-8",
                "-H", "X-Upload-Content-Type: video/*",
                "-D", "-",
                "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status",
                "-d", &metadata.to_string(),
            ])
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to initiate upload: {e}")))?;

        let init_response = String::from_utf8_lossy(&init_output.stdout).to_string();

        // Extract the upload URL from response headers
        let upload_url = init_response.lines()
            .find(|line| line.to_lowercase().starts_with("location:"))
            .and_then(|line| line.split_once(':').map(|x| x.1))
            .map(|url| url.trim().to_string())
            .ok_or_else(|| Error::Tool(format!(
                "Failed to get upload URL from YouTube. Response: {}", init_response
            )))?;

        // Step 2: Upload the video file
        let upload_output = Command::new("curl")
            .args([
                "-s", "-X", "PUT",
                "-H", &format!("Authorization: Bearer {access_token}"),
                "-H", "Content-Type: video/*",
                "--upload-file", file,
                &upload_url,
            ])
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to upload video: {e}")))?;

        if !upload_output.status.success() {
            return Err(Error::Tool(format!(
                "Upload failed: {}",
                String::from_utf8_lossy(&upload_output.stderr)
            )));
        }

        let response: Value = serde_json::from_slice(&upload_output.stdout)
            .map_err(|e| Error::Tool(format!("Invalid upload response: {e}")))?;

        let video_id = response["id"].as_str().unwrap_or("unknown");
        let video_title = response["snippet"]["title"].as_str().unwrap_or(title);
        let privacy_status = response["status"]["privacyStatus"].as_str().unwrap_or(privacy);

        Ok(format!(
            "YouTube upload complete!\nVideo ID: {video_id}\nTitle: {video_title}\nPrivacy: {privacy_status}\nURL: https://www.youtube.com/watch?v={video_id}"
        ))
    }
}

// ---------------------------------------------------------------------------
// 2. YouTubeGetVideoTool
// ---------------------------------------------------------------------------

pub struct YouTubeGetVideoTool;

#[async_trait]
impl TalosTool for YouTubeGetVideoTool {
    fn name(&self) -> &'static str { "youtube_get_video" }
    fn description(&self) -> &'static str {
        "Get YouTube video details (title, views, status) by video ID. Requires YOUTUBE_ACCESS_TOKEN."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("video_id", "string", "YouTube video ID", true)
            .with_param("access_token", "string", "OAuth2 access token (overrides env var)", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let video_id = args["video_id"].as_str().ok_or_else(|| Error::Tool("Missing 'video_id'".into()))?;
        let access_token = get_access_token(&args)?;

        let url = format!(
            "https://www.googleapis.com/youtube/v3/videos?part=snippet,statistics,status&id={video_id}"
        );

        let output = Command::new("curl")
            .args([
                "-s",
                "-H", &format!("Authorization: Bearer {access_token}"),
                &url,
            ])
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Request failed: {e}")))?;

        let response: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Tool(format!("Invalid response: {e}")))?;

        let items = response["items"].as_array()
            .ok_or_else(|| Error::Tool("No items in response".into()))?;

        if items.is_empty() {
            return Ok(format!("Video {video_id} not found"));
        }

        let item = &items[0];
        let title = item["snippet"]["title"].as_str().unwrap_or("?");
        let views = item["statistics"]["viewCount"].as_str().unwrap_or("0");
        let likes = item["statistics"]["likeCount"].as_str().unwrap_or("0");
        let status = item["status"]["privacyStatus"].as_str().unwrap_or("?");
        let upload_status = item["status"]["uploadStatus"].as_str().unwrap_or("?");

        Ok(format!(
            "Video: {title}\nID: {video_id}\nStatus: {upload_status} / {status}\nViews: {views} | Likes: {likes}\nURL: https://www.youtube.com/watch?v={video_id}"
        ))
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
        let t = YouTubeUploadTool;
        assert_eq!(t.name(), "youtube_upload");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"file") && req.contains(&"title"));
    }

    #[test]
    fn test_upload_requires_token() {
        // Token from arg always works
        let result = get_access_token(&json!({"access_token": "test"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_from_args() {
        let result = get_access_token(&json!({"access_token": "tok123"}));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "tok123");
    }

    #[test]
    fn test_get_video_schema() {
        let t = YouTubeGetVideoTool;
        assert_eq!(t.name(), "youtube_get_video");
        let s = t.schema();
        let p = s.parameters.as_object().unwrap();
        let req: Vec<&str> = p["required"].as_array().unwrap()
            .iter().filter_map(|v| v.as_str()).collect();
        assert!(req.contains(&"video_id"));
    }

    #[test]
    fn test_upload_description() {
        let t = YouTubeUploadTool;
        assert!(t.description().contains("OAuth2"));
        assert!(t.description().contains("YOUTUBE_ACCESS_TOKEN"));
    }
}
