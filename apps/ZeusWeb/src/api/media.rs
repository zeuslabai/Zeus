// TTS, STT, uploads, images, canvas, batches, vector stores, studio

use super::*;
use wasm_bindgen::JsCast;

// TTS

pub async fn tts_synthesize(text: &str) -> Result<Vec<u8>, String> {
    let url = format!("{}{}", TTS_CONFIG.base_path, TTS_CONFIG.synthesize_path);
    let body = serde_json::json!({ "text": text });
    let json = serde_json::to_string(&body).map_err(|e| format!("TTS serialize: {}", e))?;
    let mut req = gloo_net::http::Request::post(&url)
        .header("Content-Type", "application/json");
    if let Some(auth) = auth_bearer() {
        req = req.header("Authorization", &auth);
    }
    let resp = req.body(json)
        .map_err(|e| format!("TTS build: {}", e))?
        .send()
        .await
        .map_err(|e| format!("TTS request: {}", e))?;
    if !resp.ok() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("TTS HTTP {}: {}", status, body));
    }
    let text_resp = resp.text().await.map_err(|e| format!("TTS read: {}", e))?;
    let val: serde_json::Value = serde_json::from_str(&text_resp)
        .map_err(|e| format!("TTS JSON parse: {}", e))?;
    let b64 = val.get("audio_base64").and_then(|v| v.as_str())
        .ok_or("TTS response missing audio_base64")?;
    decode_base64_audio(b64)
}

pub async fn fetch_tts_providers() -> Result<TtsProvidersResponse, String> {
    fetch_json("/v1/tts/providers").await
}

pub async fn fetch_tts_voices() -> Result<TtsVoicesResponse, String> {
    fetch_json("/v1/tts/voices").await
}

// STT

/// Transcribe audio via Whisper STT (POST /stt/inference, multipart form).
pub async fn stt_transcribe(audio: &[u8]) -> Result<String, String> {
    stt_transcribe_with_mime(audio, "audio/webm").await
}

pub async fn stt_transcribe_with_mime(audio: &[u8], mime_type: &str) -> Result<String, String> {
    let form = web_sys::FormData::new().map_err(|e| format!("FormData: {:?}", e))?;

    let uint8 = js_sys::Uint8Array::new_with_length(audio.len() as u32);
    uint8.copy_from(audio);
    let parts = js_sys::Array::new();
    parts.push(&uint8.buffer());
    let blob_opts = web_sys::BlobPropertyBag::new();
    blob_opts.set_type(mime_type);
    let blob = web_sys::Blob::new_with_buffer_source_sequence_and_options(&parts, &blob_opts)
        .map_err(|e| format!("Blob: {:?}", e))?;

    let ext = if mime_type.contains("ogg") { "ogg" }
        else if mime_type.contains("mp4") { "mp4" }
        else { "webm" };
    form.append_with_blob_and_filename("file", &blob, &format!("audio.{}", ext))
        .map_err(|e| format!("FormData append: {:?}", e))?;

    form.append_with_str("temperature", "0.0")
        .map_err(|e| format!("FormData append: {:?}", e))?;
    form.append_with_str("response_format", "json")
        .map_err(|e| format!("FormData append: {:?}", e))?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form.into());

    let request = web_sys::Request::new_with_str_and_init("/stt/inference", &opts)
        .map_err(|e| format!("Request: {:?}", e))?;

    if let Some(auth) = auth_bearer() {
        request.headers()
            .set("Authorization", &auth)
            .map_err(|e| format!("Header: {:?}", e))?;
    }

    let window = web_sys::window().ok_or("No window")?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch: {:?}", e))?;

    let resp: web_sys::Response = resp_val.dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;

    if !resp.ok() {
        let status = resp.status();
        return Err(format!("STT HTTP {}", status));
    }

    let text_promise = resp.text().map_err(|e| format!("STT text: {:?}", e))?;
    let text_val = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("STT text read: {:?}", e))?;

    let text = text_val.as_string().unwrap_or_default();
    let parsed: SttResponse = serde_json::from_str(&text)
        .map_err(|e| format!("STT parse: {}", e))?;

    Ok(parsed.text.trim().to_string())
}

// Uploads

pub async fn upload_file(file: web_sys::File, _on_progress: impl Fn(f64) + 'static) -> Result<UploadedFile, String> {
    let form_data = web_sys::FormData::new().map_err(|e| format!("FormData error: {:?}", e))?;
    form_data
        .append_with_blob("file", &file)
        .map_err(|e| format!("Append blob error: {:?}", e))?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form_data);

    let request = web_sys::Request::new_with_str_and_init("/v1/uploads", &opts)
        .map_err(|e| format!("Request: {:?}", e))?;

    if let Some(auth) = auth_bearer() {
        request.headers()
            .set("Authorization", &auth)
            .map_err(|e| format!("Header: {:?}", e))?;
    }

    let window = web_sys::window().ok_or("No window")?;
    let resp_val = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch: {:?}", e))?;

    let resp: web_sys::Response = resp_val.dyn_into()
        .map_err(|_| "Response cast failed".to_string())?;

    if !resp.ok() {
        let status = resp.status();
        let body_promise = resp.text().map_err(|e| format!("Text: {:?}", e))?;
        let body = wasm_bindgen_futures::JsFuture::from(body_promise)
            .await
            .map_err(|e| format!("Text read: {:?}", e))?
            .as_string()
            .unwrap_or_default();
        return Err(if body.is_empty() {
            format!("HTTP {}", status)
        } else {
            format!("HTTP {}: {}", status, body)
        });
    }

    let text_promise = resp.text().map_err(|e| format!("Text: {:?}", e))?;
    let text = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("Text read: {:?}", e))?
        .as_string()
        .unwrap_or_default();

    serde_json::from_str::<UploadedFile>(&text)
        .map_err(|e| format!("Parse error: {} (body: {})", e, truncate_str(&text, 200)))
}

pub async fn list_uploads() -> Result<Vec<UploadedFile>, String> {
    fetch_json("/v1/uploads").await
}

pub async fn get_upload_metadata(id: &str) -> Result<UploadedFile, String> {
    fetch_json(&format!("/v1/uploads/{}", id)).await
}

pub async fn delete_upload(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/uploads/{}", id)).await
}

// Images

pub async fn fetch_images() -> Result<ImagesListResponse, String> {
    fetch_json("/v1/images").await
}

pub async fn fetch_image(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/images/{}", id)).await
}

// Canvas

pub async fn fetch_canvas_components() -> Result<CanvasComponentsResponse, String> {
    fetch_json("/v1/canvas/components").await
}

pub async fn render_canvas(body: &serde_json::Value) -> Result<CanvasRenderResponse, String> {
    post_json("/v1/canvas/render", body).await
}

// Vector Stores

pub async fn fetch_vector_stores() -> Result<VectorStoresListResponse, String> {
    fetch_json("/v1/vector_stores").await
}

pub async fn create_vector_store(body: &serde_json::Value) -> Result<VectorStore, String> {
    post_json("/v1/vector_stores", body).await
}

pub async fn fetch_vector_store(id: &str) -> Result<VectorStore, String> {
    fetch_json(&format!("/v1/vector_stores/{}", id)).await
}

pub async fn delete_vector_store(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/vector_stores/{}", id)).await
}

pub async fn search_vector_store(id: &str, body: &serde_json::Value) -> Result<VectorSearchResponse, String> {
    post_json(&format!("/v1/vector_stores/{}/search", id), body).await
}

pub async fn add_file_to_vector_store(id: &str, body: &serde_json::Value) -> Result<VectorStoreFileResponse, String> {
    post_json(&format!("/v1/vector_stores/{}/files", id), body).await
}

pub async fn fetch_vector_store_files(id: &str) -> Result<VectorStoreFilesListResponse, String> {
    fetch_json(&format!("/v1/vector_stores/{}/files", id)).await
}

// Batches

pub async fn create_batch(body: &serde_json::Value) -> Result<BatchResponse, String> {
    post_json("/v1/batches", body).await
}

pub async fn fetch_batch(id: &str) -> Result<BatchResponse, String> {
    fetch_json(&format!("/v1/batches/{}", id)).await
}

pub async fn fetch_batch_results(id: &str) -> Result<BatchResultsResponse, String> {
    fetch_json(&format!("/v1/batches/{}/results", id)).await
}

// Blog CMS







// Agent Studio

pub async fn create_studio_session(goal: &str) -> Result<serde_json::Value, String> {
    post_json("/v1/studio/sessions", &serde_json::json!({"goal": goal, "user_id": "default"})).await
}

pub async fn fetch_studio_sessions() -> Result<StudioSessionsResponse, String> {
    fetch_json("/v1/studio/sessions").await
}

pub async fn fetch_studio_session(id: &str) -> Result<StudioSession, String> {
    fetch_json(&format!("/v1/studio/sessions/{}", id)).await
}

pub async fn delete_studio_session(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/studio/sessions/{}", id)).await
}

pub async fn drive_studio_session(id: &str, approved: bool) -> Result<serde_json::Value, String> {
    post_json(
        &format!("/v1/studio/sessions/{}/drive", id),
        &serde_json::json!({"approved": approved}),
    ).await
}

pub async fn fetch_studio_stats() -> Result<StudioStats, String> {
    fetch_json("/v1/studio/stats").await
}

pub async fn fetch_active_studio_sessions() -> Result<StudioSessionsResponse, String> {
    fetch_json("/v1/studio/sessions/active").await
}
