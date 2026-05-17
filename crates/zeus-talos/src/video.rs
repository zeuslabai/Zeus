//! Video generation tools
//!
//! Text-to-video generation via ComfyUI + AnimateDiff backend or cloud APIs.
//! URL is configurable via `ZEUS_VIDEO_GEN_URL` env var, `[video_gen].url`
//! in config.toml, or `base_url` tool argument.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

/// Default video generation API base URL (local ComfyUI; override with ZEUS_VIDEO_GEN_URL env var).
const DEFAULT_BASE_URL: &str = "http://localhost:8188";

/// Default video duration in seconds.
const DEFAULT_DURATION_SECS: i64 = 4;

/// Default frames per second.
const DEFAULT_FPS: i64 = 24;

/// Get the video gen API base URL from args, env, or default.
fn get_base_url(args: &Value) -> String {
    args.get("base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("ZEUS_VIDEO_GEN_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
        })
}

/// Make an HTTP request to the video generation API via curl.
async fn video_api(
    base_url: &str,
    endpoint: &str,
    method: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}{}", base_url, endpoint);

    let mut cmd = Command::new("curl");
    cmd.arg("-s")
        .arg("-X")
        .arg(method)
        .arg("-H")
        .arg("Content-Type: application/json");

    if let Some(b) = body {
        cmd.arg("-d").arg(b.to_string());
    }

    cmd.arg(&url);

    let output = cmd
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to call video gen API: {}", e)))?;

    if !output.status.success() {
        return Err(Error::Tool(format!(
            "curl error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Tool(format!("Invalid JSON response: {}", e)))?;

    Ok(response)
}

// ---------------------------------------------------------------------------
// 1. VideoGenerateTool
// ---------------------------------------------------------------------------

/// Generate a video from a text prompt using AnimateDiff / ComfyUI.
pub struct VideoGenerateTool;

#[async_trait]
impl TalosTool for VideoGenerateTool {
    fn name(&self) -> &'static str {
        "video_generate"
    }
    fn description(&self) -> &'static str {
        "Generate a video from a text prompt using AI (AnimateDiff / ComfyUI)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "prompt",
                "string",
                "Text description of the video to generate",
                true,
            )
            .with_param(
                "negative_prompt",
                "string",
                "What to avoid in the video",
                false,
            )
            .with_param(
                "duration",
                "integer",
                "Video duration in seconds (2-10, default 4)",
                false,
            )
            .with_param("fps", "integer", "Frames per second (default 24)", false)
            .with_param("width", "integer", "Video width (default 512)", false)
            .with_param("height", "integer", "Video height (default 512)", false)
            .with_param(
                "model",
                "string",
                "Model name (default: animatediff)",
                false,
            )
            .with_param(
                "seed",
                "integer",
                "Random seed for reproducibility (-1 for random)",
                false,
            )
            .with_param(
                "base_url",
                "string",
                "API base URL (env: ZEUS_VIDEO_GEN_URL, default .249:8188)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'prompt'".to_string()))?;

        let duration = args
            .get("duration")
            .and_then(|v| v.as_i64())
            .unwrap_or(DEFAULT_DURATION_SECS)
            .clamp(2, 10);
        let fps = args
            .get("fps")
            .and_then(|v| v.as_i64())
            .unwrap_or(DEFAULT_FPS);
        let num_frames = duration * fps;

        let mut body = json!({
            "prompt": {
                "positive": prompt,
                "width": args.get("width").and_then(|v| v.as_i64()).unwrap_or(512),
                "height": args.get("height").and_then(|v| v.as_i64()).unwrap_or(512),
                "num_frames": num_frames,
                "fps": fps,
                "model": args.get("model").and_then(|v| v.as_str()).unwrap_or("animatediff"),
            }
        });

        if let Some(neg) = args.get("negative_prompt").and_then(|v| v.as_str()) {
            body["prompt"]["negative"] = json!(neg);
        }
        if let Some(seed) = args.get("seed").and_then(|v| v.as_i64()) {
            body["prompt"]["seed"] = json!(seed);
        }

        let result = video_api(&base_url, "/api/prompt", "POST", Some(&body)).await?;

        // ComfyUI returns a prompt_id for tracking
        if let Some(prompt_id) = result.get("prompt_id").and_then(|v| v.as_str()) {
            Ok(format!(
                "Video generation queued. Job ID: {}\nDuration: {}s, {} frames at {}fps\nUse video_check_status with this job ID to track progress.",
                prompt_id, duration, num_frames, fps
            ))
        } else {
            Ok(serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "Video generation submitted (check API response)".to_string()))
        }
    }
}

// ---------------------------------------------------------------------------
// 2. VideoCheckStatusTool
// ---------------------------------------------------------------------------

/// Check the status of a video generation job.
pub struct VideoCheckStatusTool;

#[async_trait]
impl TalosTool for VideoCheckStatusTool {
    fn name(&self) -> &'static str {
        "video_check_status"
    }
    fn description(&self) -> &'static str {
        "Check the status of a video generation job"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "job_id",
                "string",
                "Job/prompt ID to check status for",
                true,
            )
            .with_param(
                "base_url",
                "string",
                "API base URL (env: ZEUS_VIDEO_GEN_URL, default .249:8188)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);
        let job_id = args
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'job_id'".to_string()))?;

        let endpoint = format!("/api/history/{}", job_id);
        let result = video_api(&base_url, &endpoint, "GET", None).await?;

        // Parse ComfyUI history response
        if let Some(job_data) = result.get(job_id) {
            if let Some(outputs) = job_data.get("outputs") {
                // Check for video output nodes
                let mut video_urls = Vec::new();
                if let Some(obj) = outputs.as_object() {
                    for (_node_id, node_output) in obj {
                        if let Some(gifs) = node_output.get("gifs").and_then(|g| g.as_array()) {
                            for gif in gifs {
                                if let Some(filename) = gif.get("filename").and_then(|f| f.as_str())
                                {
                                    let subfolder =
                                        gif.get("subfolder").and_then(|s| s.as_str()).unwrap_or("");
                                    let view_url = if subfolder.is_empty() {
                                        format!("{}/view?filename={}", base_url, filename)
                                    } else {
                                        format!(
                                            "{}/view?filename={}&subfolder={}",
                                            base_url, filename, subfolder
                                        )
                                    };
                                    video_urls.push(view_url);
                                }
                            }
                        }
                        // Also check for "videos" key (some workflows use this)
                        if let Some(videos) = node_output.get("videos").and_then(|v| v.as_array()) {
                            for vid in videos {
                                if let Some(filename) = vid.get("filename").and_then(|f| f.as_str())
                                {
                                    video_urls
                                        .push(format!("{}/view?filename={}", base_url, filename));
                                }
                            }
                        }
                    }
                }

                if !video_urls.is_empty() {
                    return Ok(format!(
                        "Video generation complete! {} output(s):\n{}",
                        video_urls.len(),
                        video_urls.join("\n")
                    ));
                }

                return Ok(format!(
                    "Job {} completed. Output:\n{}",
                    job_id,
                    serde_json::to_string_pretty(outputs)
                        .unwrap_or_else(|_| "Unable to format output".to_string())
                ));
            }

            // Job exists but no outputs yet — still running
            let status = job_data
                .get("status")
                .and_then(|s| s.get("status_str"))
                .and_then(|s| s.as_str())
                .unwrap_or("processing");
            return Ok(format!("Job {} status: {}", job_id, status));
        }

        // Job not found in history — check queue
        let queue_result = video_api(&base_url, "/api/queue", "GET", None).await?;
        let running = queue_result
            .get("queue_running")
            .and_then(|q| q.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let pending = queue_result
            .get("queue_pending")
            .and_then(|q| q.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        Ok(format!(
            "Job {} not found in history. Queue: {} running, {} pending",
            job_id, running, pending
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_generate_schema() {
        let tool = VideoGenerateTool;
        assert_eq!(tool.name(), "video_generate");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"prompt"));
    }

    #[test]
    fn test_video_check_status_schema() {
        let tool = VideoCheckStatusTool;
        assert_eq!(tool.name(), "video_check_status");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"job_id"));
    }

    #[test]
    fn test_default_base_url() {
        let url = get_base_url(&json!({}));
        assert!(!url.is_empty());
    }

    #[test]
    fn test_custom_base_url() {
        let url = get_base_url(&json!({"base_url": "http://custom:9999"}));
        assert_eq!(url, "http://custom:9999");
    }

    #[test]
    fn test_video_generate_description() {
        let tool = VideoGenerateTool;
        assert!(tool.description().contains("video"));
    }

    #[test]
    fn test_video_check_status_description() {
        let tool = VideoCheckStatusTool;
        assert!(tool.description().contains("status"));
    }
}
