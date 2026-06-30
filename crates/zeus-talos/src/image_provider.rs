//! Pluggable image generation providers
//!
//! Supports multiple backends: OpenAI DALL-E, Automatic1111, ComfyUI,
//! Fooocus, and any OpenAI-compatible image API. Users configure the
//! provider via `[image_gen]` in config.toml or `ZEUS_IMAGE_GEN_*` env vars.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zeus_core::{Error, ImageGenConfig, ImageGenProviderType, Result};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Unified image generation request
#[derive(Debug, Clone, Serialize)]
pub struct ImageRequest {
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub width: u32,
    pub height: u32,
    pub model: Option<String>,
    pub style: Option<String>,
    pub n: u32,
    pub steps: Option<u32>,
    pub seed: Option<i64>,
}

impl Default for ImageRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            negative_prompt: None,
            width: 1024,
            height: 1024,
            model: None,
            style: None,
            n: 1,
            steps: None,
            seed: None,
        }
    }
}

/// A single generated image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedImage {
    /// Base64-encoded image data (PNG)
    pub base64: String,
    /// URL if the backend returned one
    pub url: Option<String>,
    /// Revised prompt (OpenAI DALL-E 3 returns this)
    pub revised_prompt: Option<String>,
}

/// Unified image generation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResponse {
    pub images: Vec<GeneratedImage>,
    /// Provider name that generated the images
    pub provider: String,
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Trait for pluggable image generation backends
#[async_trait]
pub trait ImageProvider: Send + Sync {
    /// Generate image(s) from a text prompt
    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse>;
    /// Provider display name
    fn name(&self) -> &str;
    /// Check if the backend is reachable
    async fn health_check(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create the appropriate provider from config
pub fn create_provider(config: &ImageGenConfig) -> Result<Box<dyn ImageProvider>> {
    match config.provider {
        ImageGenProviderType::OpenAi => Ok(Box::new(OpenAiProvider::new(config)?)),
        ImageGenProviderType::OpenAiCompatible => {
            Ok(Box::new(OpenAiCompatibleProvider::new(config)?))
        }
        ImageGenProviderType::Automatic1111 => Ok(Box::new(Automatic1111Provider::new(config))),
        ImageGenProviderType::ComfyUi => Ok(Box::new(ComfyUiProvider::new(config))),
        ImageGenProviderType::Fooocus => Ok(Box::new(FooocusProvider::new(config))),
    }
}

// ---------------------------------------------------------------------------
// OpenAI DALL-E Provider
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: &ImageGenConfig) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                Error::config(
                    "OpenAI image provider requires an API key. \
                     Set `api_key` in [image_gen] or ZEUS_IMAGE_GEN_API_KEY / OPENAI_API_KEY env var.",
                )
            })?;

        let url = if config.url.contains("localhost") || config.url.contains("127.0.0.1") {
            "https://api.openai.com".to_string()
        } else {
            config.url.clone()
        };

        let model = config
            .model
            .clone()
            .unwrap_or_else(|| "dall-e-3".to_string());

        Ok(Self {
            url,
            api_key,
            model,
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl ImageProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse> {
        let size = format!("{}x{}", request.width, request.height);
        let mut body = json!({
            "model": self.model,
            "prompt": request.prompt,
            "n": request.n,
            "size": size,
            "response_format": "b64_json",
        });

        if let Some(ref style) = request.style {
            body["style"] = json!(style);
        }

        let resp = self
            .client
            .post(format!("{}/v1/images/generations", self.url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| Error::tool(format!("OpenAI image request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::tool(format!(
                "OpenAI image API returned {status}: {text}"
            )));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| Error::tool(format!("Invalid OpenAI response: {e}")))?;

        let images = resp_json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|img| GeneratedImage {
                        base64: img
                            .get("b64_json")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        url: img.get("url").and_then(|v| v.as_str()).map(String::from),
                        revised_prompt: img
                            .get("revised_prompt")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ImageResponse {
            images,
            provider: "openai".to_string(),
        })
    }

    async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/v1/models", self.url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// OpenAI-Compatible Provider (custom endpoint)
// ---------------------------------------------------------------------------

pub struct OpenAiCompatibleProvider {
    url: String,
    api_key: Option<String>,
    model: String,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: &ImageGenConfig) -> Result<Self> {
        Ok(Self {
            url: config.url.clone(),
            api_key: config.api_key.clone(),
            model: config
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl ImageProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        "openai_compatible"
    }

    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse> {
        let size = format!("{}x{}", request.width, request.height);
        let body = json!({
            "model": self.model,
            "prompt": request.prompt,
            "n": request.n,
            "size": size,
            "response_format": "b64_json",
        });

        let mut req_builder = self
            .client
            .post(format!("{}/v1/images/generations", self.url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(120));

        if let Some(ref key) = self.api_key {
            req_builder = req_builder.header("Authorization", format!("Bearer {key}"));
        }

        let resp = req_builder
            .send()
            .await
            .map_err(|e| Error::tool(format!("Image API request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::tool(format!("Image API returned {status}: {text}")));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| Error::tool(format!("Invalid response: {e}")))?;

        let images = resp_json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|img| GeneratedImage {
                        base64: img
                            .get("b64_json")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        url: img.get("url").and_then(|v| v.as_str()).map(String::from),
                        revised_prompt: img
                            .get("revised_prompt")
                            .and_then(|v| v.as_str())
                            .map(String::from),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ImageResponse {
            images,
            provider: "openai_compatible".to_string(),
        })
    }

    async fn health_check(&self) -> bool {
        let mut req = self
            .client
            .get(format!("{}/v1/models", self.url))
            .timeout(std::time::Duration::from_secs(5));
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
        req.send().await.is_ok()
    }
}

// ---------------------------------------------------------------------------
// Automatic1111 (Stable Diffusion WebUI) Provider
// ---------------------------------------------------------------------------

pub struct Automatic1111Provider {
    url: String,
    client: reqwest::Client,
}

impl Automatic1111Provider {
    pub fn new(config: &ImageGenConfig) -> Self {
        Self {
            url: config.url.clone(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ImageProvider for Automatic1111Provider {
    fn name(&self) -> &str {
        "automatic1111"
    }

    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse> {
        let mut body = json!({
            "prompt": request.prompt,
            "width": request.width,
            "height": request.height,
            "batch_size": request.n,
        });

        if let Some(ref neg) = request.negative_prompt {
            body["negative_prompt"] = json!(neg);
        }
        if let Some(steps) = request.steps {
            body["steps"] = json!(steps);
        }
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }

        let resp = self
            .client
            .post(format!("{}/sdapi/v1/txt2img", self.url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| Error::tool(format!("Automatic1111 request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::tool(format!(
                "Automatic1111 returned {status}: {text}"
            )));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| Error::tool(format!("Invalid Automatic1111 response: {e}")))?;

        let images = resp_json
            .get("images")
            .and_then(|i| i.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|img| GeneratedImage {
                        base64: img.as_str().unwrap_or("").to_string(),
                        url: None,
                        revised_prompt: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ImageResponse {
            images,
            provider: "automatic1111".to_string(),
        })
    }

    async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/sdapi/v1/sd-models", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// ComfyUI Provider
// ---------------------------------------------------------------------------

pub struct ComfyUiProvider {
    url: String,
    client: reqwest::Client,
}

impl ComfyUiProvider {
    pub fn new(config: &ImageGenConfig) -> Self {
        Self {
            url: config.url.clone(),
            client: reqwest::Client::new(),
        }
    }

    /// Build a minimal ComfyUI workflow JSON for text-to-image
    fn build_workflow(&self, request: &ImageRequest) -> Value {
        json!({
            "prompt": {
                "3": {
                    "class_type": "KSampler",
                    "inputs": {
                        "seed": request.seed.unwrap_or(-1),
                        "steps": request.steps.unwrap_or(20),
                        "cfg": 7.0,
                        "sampler_name": "euler",
                        "scheduler": "normal",
                        "denoise": 1.0,
                        "model": ["4", 0],
                        "positive": ["6", 0],
                        "negative": ["7", 0],
                        "latent_image": ["5", 0]
                    }
                },
                "4": {
                    "class_type": "CheckpointLoaderSimple",
                    "inputs": {
                        "ckpt_name": request.model.as_deref().unwrap_or("sd_xl_base_1.0.safetensors")
                    }
                },
                "5": {
                    "class_type": "EmptyLatentImage",
                    "inputs": {
                        "width": request.width,
                        "height": request.height,
                        "batch_size": request.n
                    }
                },
                "6": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": request.prompt,
                        "clip": ["4", 1]
                    }
                },
                "7": {
                    "class_type": "CLIPTextEncode",
                    "inputs": {
                        "text": request.negative_prompt.as_deref().unwrap_or(""),
                        "clip": ["4", 1]
                    }
                },
                "8": {
                    "class_type": "VAEDecode",
                    "inputs": {
                        "samples": ["3", 0],
                        "vae": ["4", 2]
                    }
                },
                "9": {
                    "class_type": "SaveImage",
                    "inputs": {
                        "filename_prefix": "zeus",
                        "images": ["8", 0]
                    }
                }
            }
        })
    }
}

#[async_trait]
impl ImageProvider for ComfyUiProvider {
    fn name(&self) -> &str {
        "comfyui"
    }

    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse> {
        let workflow = self.build_workflow(request);

        let resp = self
            .client
            .post(format!("{}/api/prompt", self.url))
            .json(&workflow)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| Error::tool(format!("ComfyUI request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::tool(format!("ComfyUI returned {status}: {text}")));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| Error::tool(format!("Invalid ComfyUI response: {e}")))?;

        // ComfyUI returns a prompt_id — images must be fetched from history
        let prompt_id = resp_json
            .get("prompt_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if prompt_id.is_empty() {
            return Err(Error::tool("ComfyUI returned no prompt_id".to_string()));
        }

        // Poll history for completed images (wait up to 60s)
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let history_resp = self
                .client
                .get(format!("{}/api/history/{}", self.url, prompt_id))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;

            if let Ok(hr) = history_resp
                && let Ok(history) = hr.json::<Value>().await
                && let Some(entry) = history.get(prompt_id)
                && let Some(outputs) = entry.get("outputs")
            {
                let mut images = Vec::new();
                for (_node_id, output) in outputs.as_object().into_iter().flatten() {
                    if let Some(img_arr) = output.get("images").and_then(|v| v.as_array()) {
                        for img in img_arr {
                            let filename =
                                img.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                            let subfolder =
                                img.get("subfolder").and_then(|v| v.as_str()).unwrap_or("");
                            let img_type =
                                img.get("type").and_then(|v| v.as_str()).unwrap_or("output");

                            let view_url = format!(
                                "{}/view?filename={}&subfolder={}&type={}",
                                self.url, filename, subfolder, img_type
                            );

                            if let Ok(img_resp) = self.client.get(&view_url).send().await
                                && let Ok(bytes) = img_resp.bytes().await
                            {
                                use base64::Engine;
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                                images.push(GeneratedImage {
                                    base64: b64,
                                    url: Some(view_url),
                                    revised_prompt: None,
                                });
                            }
                        }
                    }
                }
                if !images.is_empty() {
                    return Ok(ImageResponse {
                        images,
                        provider: "comfyui".to_string(),
                    });
                }
            }
        }

        Err(Error::tool(format!(
            "ComfyUI generation timed out for prompt_id: {prompt_id}"
        )))
    }

    async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/api/system_stats", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// Fooocus Provider
// ---------------------------------------------------------------------------

pub struct FooocusProvider {
    url: String,
    client: reqwest::Client,
}

impl FooocusProvider {
    pub fn new(config: &ImageGenConfig) -> Self {
        Self {
            url: config.url.clone(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ImageProvider for FooocusProvider {
    fn name(&self) -> &str {
        "fooocus"
    }

    async fn generate(&self, request: &ImageRequest) -> Result<ImageResponse> {
        let mut body = json!({
            "prompt": request.prompt,
            "width": request.width,
            "height": request.height,
            "image_number": request.n,
        });

        if let Some(ref neg) = request.negative_prompt {
            body["negative_prompt"] = json!(neg);
        }
        if let Some(steps) = request.steps {
            body["steps"] = json!(steps);
        }
        if let Some(seed) = request.seed {
            body["seed"] = json!(seed);
        }
        if let Some(ref style) = request.style {
            body["style"] = json!(style);
        }

        let resp = self
            .client
            .post(format!("{}/v1/generation/text-to-image", self.url))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| Error::tool(format!("Fooocus request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::tool(format!("Fooocus returned {status}: {text}")));
        }

        let resp_json: Value = resp
            .json()
            .await
            .map_err(|e| Error::tool(format!("Invalid Fooocus response: {e}")))?;

        // Fooocus may return an array of images or an object with image data
        let images = if let Some(arr) = resp_json.as_array() {
            arr.iter()
                .map(|img| GeneratedImage {
                    base64: img
                        .get("base64")
                        .or_else(|| img.get("image"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: img
                        .get("url")
                        .or_else(|| img.get("image_url"))
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    revised_prompt: None,
                })
                .collect()
        } else {
            vec![GeneratedImage {
                base64: resp_json
                    .get("base64")
                    .or_else(|| resp_json.get("image"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: resp_json
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                revised_prompt: None,
            }]
        };

        Ok(ImageResponse {
            images,
            provider: "fooocus".to_string(),
        })
    }

    async fn health_check(&self) -> bool {
        self.client
            .get(format!("{}/v1/engines/all-models", self.url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_fooocus_provider() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::Fooocus,
            ..Default::default()
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "fooocus");
    }

    #[test]
    fn test_create_automatic1111_provider() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::Automatic1111,
            url: "http://localhost:7860".to_string(),
            ..Default::default()
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "automatic1111");
    }

    #[test]
    fn test_create_comfyui_provider() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::ComfyUi,
            url: "http://localhost:8188".to_string(),
            ..Default::default()
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "comfyui");
    }

    #[test]
    fn test_create_openai_provider() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::OpenAi,
            url: "https://api.openai.com".to_string(),
            api_key: Some("sk-test-key".to_string()),
            model: Some("dall-e-3".to_string()),
            ..Default::default()
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_create_openai_provider_no_key_fails() {
        // Temporarily clear env vars to ensure failure
        let config = ImageGenConfig {
            provider: ImageGenProviderType::OpenAi,
            url: "https://api.openai.com".to_string(),
            api_key: None,
            model: None,
            ..Default::default()
        };
        // May succeed if OPENAI_API_KEY env var is set, otherwise should fail
        let result = create_provider(&config);
        if std::env::var("OPENAI_API_KEY").is_err() {
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_create_openai_compatible_provider() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::OpenAiCompatible,
            url: "http://my-custom-api.example.com".to_string(),
            api_key: Some("custom-key".to_string()),
            model: Some("my-model".to_string()),
            ..Default::default()
        };
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "openai_compatible");
    }

    #[test]
    fn test_create_openai_compatible_no_key_ok() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::OpenAiCompatible,
            url: "http://localhost:9999".to_string(),
            api_key: None,
            ..Default::default()
        };
        // OpenAI-compatible does not require an API key
        let provider = create_provider(&config).unwrap();
        assert_eq!(provider.name(), "openai_compatible");
    }

    #[test]
    fn test_image_request_default() {
        let req = ImageRequest::default();
        assert_eq!(req.width, 1024);
        assert_eq!(req.height, 1024);
        assert_eq!(req.n, 1);
        assert!(req.prompt.is_empty());
    }

    #[test]
    fn test_provider_type_default() {
        let provider_type = ImageGenProviderType::default();
        // Without ZEUS_IMAGE_GEN_PROVIDER set, defaults to Fooocus
        if std::env::var("ZEUS_IMAGE_GEN_PROVIDER").is_err() {
            assert_eq!(provider_type, ImageGenProviderType::Fooocus);
        }
    }

    #[test]
    fn test_comfyui_workflow_structure() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::ComfyUi,
            url: "http://localhost:8188".to_string(),
            ..Default::default()
        };
        let provider = ComfyUiProvider::new(&config);
        let request = ImageRequest {
            prompt: "a cat".to_string(),
            width: 512,
            height: 512,
            ..Default::default()
        };
        let workflow = provider.build_workflow(&request);
        assert!(workflow.get("prompt").is_some());
        let prompt = workflow.get("prompt").unwrap();
        // Should have KSampler, CheckpointLoader, EmptyLatentImage, etc.
        assert!(prompt.get("3").is_some()); // KSampler
        assert!(prompt.get("4").is_some()); // CheckpointLoader
        assert!(prompt.get("5").is_some()); // EmptyLatentImage
    }

    #[tokio::test]
    async fn test_fooocus_health_check_unreachable() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::Fooocus,
            url: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        };
        let provider = FooocusProvider::new(&config);
        assert!(!provider.health_check().await);
    }

    #[tokio::test]
    async fn test_automatic1111_health_check_unreachable() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::Automatic1111,
            url: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        };
        let provider = Automatic1111Provider::new(&config);
        assert!(!provider.health_check().await);
    }

    #[tokio::test]
    async fn test_comfyui_health_check_unreachable() {
        let config = ImageGenConfig {
            provider: ImageGenProviderType::ComfyUi,
            url: "http://127.0.0.1:1".to_string(),
            ..Default::default()
        };
        let provider = ComfyUiProvider::new(&config);
        assert!(!provider.health_check().await);
    }
}
