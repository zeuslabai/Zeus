//! Fooocus AI image generation tools
//!
//! Connects to a Fooocus/ComfyUI backend for AI image generation using SDXL.
//! URL is configurable via `FOOOCUS_API_URL` env var or `base_url` arg.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

/// Default Fooocus API base URL (local dev; override with FOOOCUS_API_URL env var).
const DEFAULT_BASE_URL: &str = "http://localhost:8888";

/// Get the Fooocus API base URL from args, env, or default.
///
/// Resolution order: tool arg `base_url` → `ZEUS_FOOOCUS_URL` env →
/// `FOOOCUS_API_URL` env (legacy) → compiled default.
fn get_base_url(args: &Value) -> String {
    args.get("base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("ZEUS_FOOOCUS_URL")
                .or_else(|_| std::env::var("FOOOCUS_API_URL"))
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
        })
}

/// Make an HTTP request to the Fooocus API via curl
async fn fooocus_api(
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
        .map_err(|e| Error::Tool(format!("Failed to call Fooocus API: {}", e)))?;

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
// 1. FooocusGenerateTool
// ---------------------------------------------------------------------------

/// Generate an image using Fooocus (SDXL Turbo)
pub struct FooocusGenerateTool;

#[async_trait]
impl TalosTool for FooocusGenerateTool {
    fn name(&self) -> &'static str {
        "fooocus_generate"
    }
    fn description(&self) -> &'static str {
        "Generate an AI image using Fooocus (SDXL Turbo)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "prompt",
                "string",
                "Text description of the image to generate",
                true,
            )
            .with_param(
                "negative_prompt",
                "string",
                "What to avoid in the image",
                false,
            )
            .with_param("width", "integer", "Image width (default 1024)", false)
            .with_param("height", "integer", "Image height (default 1024)", false)
            .with_param(
                "steps",
                "integer",
                "Number of inference steps (default 4 for turbo)",
                false,
            )
            .with_param(
                "seed",
                "integer",
                "Random seed for reproducibility (-1 for random)",
                false,
            )
            .with_param("style", "string", "Style preset name", false)
            .with_param(
                "base_url",
                "string",
                "API base URL (env: FOOOCUS_API_URL, default localhost:8888)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'prompt'".to_string()))?;

        let mut body = json!({
            "prompt": prompt,
            "width": args.get("width").and_then(|v| v.as_i64()).unwrap_or(1024),
            "height": args.get("height").and_then(|v| v.as_i64()).unwrap_or(1024),
            "steps": args.get("steps").and_then(|v| v.as_i64()).unwrap_or(4),
        });

        if let Some(neg) = args.get("negative_prompt").and_then(|v| v.as_str()) {
            body["negative_prompt"] = json!(neg);
        }
        if let Some(seed) = args.get("seed").and_then(|v| v.as_i64()) {
            body["seed"] = json!(seed);
        }
        if let Some(style) = args.get("style").and_then(|v| v.as_str()) {
            body["style"] = json!(style);
        }

        let result = fooocus_api(
            &base_url,
            "/v1/generation/text-to-image",
            "POST",
            Some(&body),
        )
        .await?;

        // Response typically contains image URLs or base64
        if let Some(images) = result.as_array() {
            let urls: Vec<String> = images
                .iter()
                .filter_map(|img| {
                    img.get("url")
                        .and_then(|v| v.as_str())
                        .or_else(|| img.get("image_url").and_then(|v| v.as_str()))
                        .map(|s| s.to_string())
                })
                .collect();
            if !urls.is_empty() {
                return Ok(format!(
                    "Generated {} image(s):\n{}",
                    urls.len(),
                    urls.join("\n")
                ));
            }
        }

        Ok(serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "Image generated (check API response)".to_string()))
    }
}

// ---------------------------------------------------------------------------
// 2. FooocusBatchGenerateTool
// ---------------------------------------------------------------------------

/// Generate multiple images in a batch
pub struct FooocusBatchGenerateTool;

#[async_trait]
impl TalosTool for FooocusBatchGenerateTool {
    fn name(&self) -> &'static str {
        "fooocus_batch_generate"
    }
    fn description(&self) -> &'static str {
        "Generate multiple AI images in a batch"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("prompt", "string", "Text description of the images", true)
            .with_param(
                "count",
                "integer",
                "Number of images to generate (1-8, default 4)",
                false,
            )
            .with_param("negative_prompt", "string", "What to avoid", false)
            .with_param("width", "integer", "Image width (default 1024)", false)
            .with_param("height", "integer", "Image height (default 1024)", false)
            .with_param(
                "base_url",
                "string",
                "API base URL (env: FOOOCUS_API_URL, default localhost:8888)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'prompt'".to_string()))?;
        let count = args
            .get("count")
            .and_then(|v| v.as_i64())
            .unwrap_or(4)
            .clamp(1, 8);

        let mut body = json!({
            "prompt": prompt,
            "image_number": count,
            "width": args.get("width").and_then(|v| v.as_i64()).unwrap_or(1024),
            "height": args.get("height").and_then(|v| v.as_i64()).unwrap_or(1024),
        });

        if let Some(neg) = args.get("negative_prompt").and_then(|v| v.as_str()) {
            body["negative_prompt"] = json!(neg);
        }

        let result = fooocus_api(
            &base_url,
            "/v1/generation/text-to-image",
            "POST",
            Some(&body),
        )
        .await?;

        Ok(serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| format!("Batch of {} images queued", count)))
    }
}

// ---------------------------------------------------------------------------
// 3. FooocusCheckStatusTool
// ---------------------------------------------------------------------------

/// Check the status of a Fooocus generation task
pub struct FooocusCheckStatusTool;

#[async_trait]
impl TalosTool for FooocusCheckStatusTool {
    fn name(&self) -> &'static str {
        "fooocus_check_status"
    }
    fn description(&self) -> &'static str {
        "Check the status of the Fooocus image generation queue"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "job_id",
                "string",
                "Job ID to check (optional, shows queue status if omitted)",
                false,
            )
            .with_param(
                "base_url",
                "string",
                "API base URL (env: FOOOCUS_API_URL, default localhost:8888)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);

        let endpoint = if let Some(job_id) = args.get("job_id").and_then(|v| v.as_str()) {
            format!("/v1/generation/query-job?job_id={}", job_id)
        } else {
            "/v1/generation/job-queue".to_string()
        };

        let result = fooocus_api(&base_url, &endpoint, "GET", None).await?;
        Ok(serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "Unable to parse status".to_string()))
    }
}

// ---------------------------------------------------------------------------
// 4. FooocusGetModelsTool
// ---------------------------------------------------------------------------

/// List available Fooocus models/styles
pub struct FooocusGetModelsTool;

#[async_trait]
impl TalosTool for FooocusGetModelsTool {
    fn name(&self) -> &'static str {
        "fooocus_get_models"
    }
    fn description(&self) -> &'static str {
        "List available AI models and styles on the Fooocus server"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "base_url",
            "string",
            "API base URL (env: FOOOCUS_API_URL, default localhost:8888)",
            false,
        )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let base_url = get_base_url(&args);
        let result = fooocus_api(&base_url, "/v1/engines/all-models", "GET", None).await?;
        Ok(serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "Unable to list models".to_string()))
    }
}

// ---------------------------------------------------------------------------
// 5. AnalyzeImageTool
// ---------------------------------------------------------------------------

/// Analyze an image by passing it to Claude Code's native multimodal vision.
///
/// Returns an MCP image content block — Claude Code sees the image directly
/// in the tool response, no separate API key or HTTP call needed.
pub struct AnalyzeImageTool;

#[async_trait]
impl TalosTool for AnalyzeImageTool {
    fn name(&self) -> &'static str {
        "analyze_image"
    }
    fn description(&self) -> &'static str {
        "Analyze an image using AI vision (describe, OCR, or answer questions about an image)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("image_path", "string", "Local path to the image file", true)
            .with_param(
                "prompt",
                "string",
                "Question or instruction about the image (default: 'Describe this image in detail')",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let image_path = args
            .get("image_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'image_path'".to_string()))?;
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("Describe this image in detail");

        // Verify image exists
        if !std::path::Path::new(image_path).exists() {
            return Err(Error::Tool(format!("Image not found: {}", image_path)));
        }

        // Read and base64-encode the image
        let image_data = std::fs::read(image_path)
            .map_err(|e| Error::Tool(format!("Failed to read image: {}", e)))?;

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&image_data);

        // Determine MIME type from extension
        let media_type = if image_path.ends_with(".png") {
            "image/png"
        } else if image_path.ends_with(".gif") {
            "image/gif"
        } else if image_path.ends_with(".webp") {
            "image/webp"
        } else {
            "image/jpeg"
        };

        // Return MCP content with image block — Claude Code sees this natively.
        // No separate API call or API key needed; the MCP handler detects the
        // _mcp_content marker and returns proper image content blocks.
        let mcp_content = json!({
            "_mcp_content": [
                {
                    "type": "image",
                    "data": b64,
                    "mimeType": media_type
                },
                {
                    "type": "text",
                    "text": format!("{}\n\n(Image: {})", prompt, image_path)
                }
            ]
        });

        Ok(mcp_content.to_string())
    }
}

// ---------------------------------------------------------------------------
// 6. ImageGenerateTool (unified, provider-agnostic)
// ---------------------------------------------------------------------------

/// Generate images using the configured provider (OpenAI, A1111, ComfyUI, Fooocus, etc.)
///
/// Routes to whichever backend the user has configured in `[image_gen]` config
/// or via `ZEUS_IMAGE_GEN_*` env vars. Zero hardcoded backends.
pub struct ImageGenerateTool;

#[async_trait]
impl TalosTool for ImageGenerateTool {
    fn name(&self) -> &'static str {
        "image_generate"
    }
    fn description(&self) -> &'static str {
        "Generate an AI image using the configured provider (OpenAI DALL-E, Automatic1111, ComfyUI, Fooocus, or any OpenAI-compatible API)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "prompt",
                "string",
                "Text description of the image to generate",
                true,
            )
            .with_param(
                "negative_prompt",
                "string",
                "What to avoid in the image (not supported by all providers)",
                false,
            )
            .with_param(
                "width",
                "integer",
                "Image width in pixels (default 1024)",
                false,
            )
            .with_param(
                "height",
                "integer",
                "Image height in pixels (default 1024)",
                false,
            )
            .with_param(
                "model",
                "string",
                "Model name (e.g., 'dall-e-3', 'sd-xl-turbo')",
                false,
            )
            .with_param("style", "string", "Style preset (provider-specific)", false)
            .with_param(
                "n",
                "integer",
                "Number of images to generate (default 1)",
                false,
            )
            .with_param(
                "steps",
                "integer",
                "Inference steps (local providers only)",
                false,
            )
            .with_param("seed", "integer", "Random seed for reproducibility", false)
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'prompt'".to_string()))?;

        let config = zeus_core::ImageGenConfig::default();
        let provider = crate::image_provider::create_provider(&config)
            .map_err(|e| Error::Tool(format!("Failed to create image provider: {e}")))?;

        let request = crate::image_provider::ImageRequest {
            prompt: prompt.to_string(),
            negative_prompt: args
                .get("negative_prompt")
                .and_then(|v| v.as_str())
                .map(String::from),
            width: args
                .get("width")
                .and_then(|v| v.as_u64())
                .unwrap_or(config.default_width as u64) as u32,
            height: args
                .get("height")
                .and_then(|v| v.as_u64())
                .unwrap_or(config.default_height as u64) as u32,
            model: args
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or(config.model.clone()),
            style: args.get("style").and_then(|v| v.as_str()).map(String::from),
            n: args.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
            steps: args.get("steps").and_then(|v| v.as_u64()).map(|v| v as u32),
            seed: args.get("seed").and_then(|v| v.as_i64()),
        };

        let response = provider.generate(&request).await?;

        if response.images.is_empty() {
            return Err(Error::Tool("No images were generated".to_string()));
        }

        let mut result_parts = vec![format!(
            "Generated {} image(s) via {} provider:",
            response.images.len(),
            response.provider,
        )];

        for (i, img) in response.images.iter().enumerate() {
            if let Some(ref url) = img.url {
                result_parts.push(format!("  Image {}: {}", i + 1, url));
            } else if !img.base64.is_empty() {
                result_parts.push(format!(
                    "  Image {}: [base64, {} bytes]",
                    i + 1,
                    img.base64.len()
                ));
            }
            if let Some(ref revised) = img.revised_prompt {
                result_parts.push(format!("  Revised prompt: {}", revised));
            }
        }

        Ok(result_parts.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fooocus_generate_schema() {
        let tool = FooocusGenerateTool;
        assert_eq!(tool.name(), "fooocus_generate");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"prompt"));
    }

    #[test]
    fn test_fooocus_batch_schema() {
        let tool = FooocusBatchGenerateTool;
        assert_eq!(tool.name(), "fooocus_batch_generate");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let props = params["properties"].as_object().expect("props");
        assert!(props.contains_key("count"));
    }

    #[test]
    fn test_fooocus_check_status_schema() {
        let tool = FooocusCheckStatusTool;
        assert_eq!(tool.name(), "fooocus_check_status");
    }

    #[test]
    fn test_fooocus_get_models_schema() {
        let tool = FooocusGetModelsTool;
        assert_eq!(tool.name(), "fooocus_get_models");
    }

    #[test]
    fn test_analyze_image_schema() {
        let tool = AnalyzeImageTool;
        assert_eq!(tool.name(), "analyze_image");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"image_path"));
        // Verify no base_url param (forge removed, uses Claude vision)
        let props = params["properties"].as_object().expect("props");
        assert!(!props.contains_key("base_url"));
    }

    #[test]
    fn test_default_base_url() {
        let url = get_base_url(&json!({}));
        // Either env var or default
        assert!(!url.is_empty());
    }
}
