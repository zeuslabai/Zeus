//! Image generation handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use tracing::{info, warn};
use crate::SharedState;
use crate::handlers::chat_handlers::ImageGenRequest;

/// POST /v1/images/generate
pub async fn generate_image(
    State(state): State<SharedState>,
    Json(body): Json<ImageGenRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let mut image_config = state_guard.config.image_gen.clone().unwrap_or_default();
    drop(state_guard);

    if let Some(ref provider_str) = body.provider {
        image_config.provider = match provider_str.to_lowercase().as_str() {
            "openai" | "dall-e" | "dalle" => zeus_core::ImageGenProviderType::OpenAi,
            "automatic1111" | "a1111" | "sd-webui" => zeus_core::ImageGenProviderType::Automatic1111,
            "comfyui" | "comfy" => zeus_core::ImageGenProviderType::ComfyUi,
            "fooocus" => zeus_core::ImageGenProviderType::Fooocus,
            "openai_compatible" | "openai-compatible" | "generic" => zeus_core::ImageGenProviderType::OpenAiCompatible,
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Unknown image provider: {provider_str}. Supported: openai, automatic1111, comfyui, fooocus, openai_compatible"),
                ));
            }
        };
    }
    if let Some(ref model) = body.model {
        image_config.model = Some(model.clone());
    }

    let width = body.width.unwrap_or(image_config.default_width);
    let height = body.height.unwrap_or(image_config.default_height);

    info!(
        provider = ?image_config.provider,
        url = %image_config.url,
        prompt = %body.prompt,
        width,
        height,
        "Generating image via provider"
    );

    let provider = zeus_talos::image_provider::create_provider(&image_config).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("Failed to initialize image provider: {e}"))
    })?;

    let request = zeus_talos::image_provider::ImageRequest {
        prompt: body.prompt.clone(),
        negative_prompt: body.negative_prompt.clone(),
        width,
        height,
        model: body.model.clone().or(image_config.model.clone()),
        style: body.style.clone(),
        n: body.n.unwrap_or(1),
        steps: body.steps,
        seed: body.seed,
    };

    let response = provider.generate(&request).await.map_err(|e| {
        warn!(error = %e, "Image generation failed");
        (StatusCode::BAD_GATEWAY, format!("Image generation failed: {e}"))
    })?;

    if response.images.is_empty() {
        return Err((StatusCode::BAD_GATEWAY, "Image provider returned no images".to_string()));
    }

    let image_base64 = &response.images[0].base64;
    let image_id = uuid::Uuid::new_v4().to_string();
    let store_path = image_config.store_path;

    if let Err(e) = tokio::fs::create_dir_all(&store_path).await {
        warn!(error = %e, "Failed to create image store directory");
    } else if !image_base64.is_empty() {
        if let Ok(image_bytes) = base64_decode(image_base64) {
            let img_path = store_path.join(format!("{image_id}.png"));
            let _ = tokio::fs::write(&img_path, &image_bytes).await;

            let meta = serde_json::json!({
                "image_id": image_id,
                "prompt": body.prompt,
                "negative_prompt": body.negative_prompt,
                "style": body.style,
                "model": body.model,
                "provider": response.provider,
                "width": width,
                "height": height,
                "images_count": response.images.len(),
                "created_at": chrono::Utc::now().to_rfc3339(),
            });
            let meta_path = store_path.join(format!("{image_id}.json"));
            if let Ok(meta_json) = serde_json::to_string_pretty(&meta) {
                let _ = tokio::fs::write(&meta_path, meta_json).await;
            }
        }
    }

    info!(image_id = %image_id, provider = %response.provider, images = response.images.len(), "Image generated successfully");

    let images_json: Vec<Value> = response.images.iter().map(|img| {
        let mut obj = json!({ "base64": img.base64 });
        if let Some(ref url) = img.url { obj["url"] = json!(url); }
        if let Some(ref revised) = img.revised_prompt { obj["revised_prompt"] = json!(revised); }
        obj
    }).collect();

    Ok(Json(json!({
        "image_id": image_id,
        "image_base64": image_base64,
        "images": images_json,
        "provider": response.provider,
        "prompt": body.prompt,
        "width": width,
        "height": height,
    })))
}

/// GET /v1/images
pub async fn list_images(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let store_path = state_guard
        .config
        .image_gen
        .as_ref()
        .map(|c| c.store_path.clone())
        .unwrap_or_else(zeus_core::ImageGenConfig::default_store_path);

    let mut images = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&store_path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Ok(content) = tokio::fs::read_to_string(&path).await
                && let Ok(meta) = serde_json::from_str::<Value>(&content)
            {
                images.push(meta);
            }
        }
    }

    Json(json!({ "images": images, "total": images.len() }))
}

/// GET /v1/images/:id
pub async fn get_image(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let store_path = state_guard
        .config
        .image_gen
        .as_ref()
        .map(|c| c.store_path.clone())
        .unwrap_or_else(zeus_core::ImageGenConfig::default_store_path);

    let meta_path = store_path.join(format!("{id}.json"));
    let meta_content = tokio::fs::read_to_string(&meta_path)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("Image not found: {id}")))?;
    let meta: Value = serde_json::from_str(&meta_content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Bad metadata: {e}")))?;

    let img_path = store_path.join(format!("{id}.png"));
    let image_base64 = if let Ok(bytes) = tokio::fs::read(&img_path).await {
        base64_encode(&bytes)
    } else {
        String::new()
    };

    let mut result = meta;
    result["image_base64"] = Value::String(image_base64);
    Ok(Json(result))
}

pub(crate) fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode error: {e}"))
}

fn base64_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input)
}
