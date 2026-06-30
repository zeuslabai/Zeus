use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::{json, Value};
use zeus_tts::TTSProvider;

pub async fn list_tts_providers() -> Json<Value> {
    Json(json!({
        "providers": [
            {"name": "elevenlabs", "status": "available", "description": "ElevenLabs high-quality TTS"},
            {"name": "openai", "status": "available", "description": "OpenAI TTS (tts-1, tts-1-hd)"},
            {"name": "edge", "status": "available", "description": "Microsoft Edge TTS (free)"},
            {"name": "local", "status": "available", "description": "Local TTS via Piper/espeak"}
        ]
    }))
}

pub async fn tts_synthesize(Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, String)> {
    use base64::Engine;

    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'text' field".to_string()))?;

    let voice = body.get("voice").and_then(|v| v.as_str()).unwrap_or("default");
    let format_str = body.get("format").and_then(|v| v.as_str()).unwrap_or("wav");
    let speed = body.get("speed").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(1.0);
    let piper_url = body.get("piper_url").and_then(|v| v.as_str()).map(|s| s.to_string());

    let audio_format = match format_str {
        "mp3" => zeus_tts::AudioFormat::Mp3,
        "opus" => zeus_tts::AudioFormat::Opus,
        _ => zeus_tts::AudioFormat::Wav,
    };

    let provider = zeus_tts::piper::PiperHttpProvider::new(piper_url);
    let response = provider
        .synthesize(text, voice, speed, audio_format)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("TTS synthesis failed: {e}")))?;

    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&response.audio);

    Ok(Json(json!({
        "status": "completed",
        "provider": response.provider,
        "voice": response.voice,
        "format": format_str,
        "audio_base64": audio_b64,
        "audio_size_bytes": response.audio.len(),
        "duration_ms": response.duration_ms,
    })))
}

pub async fn tts_synthesize_stream(
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'text' field".to_string()))?;

    let voice = body.get("voice").and_then(|v| v.as_str()).unwrap_or("default");
    let format = body.get("format").and_then(|v| v.as_str()).unwrap_or("wav");
    let speed = body.get("speed").and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(1.0);
    let piper_url = body.get("piper_url").and_then(|v| v.as_str()).map(|s| s.to_string());

    let audio_format = match format {
        "mp3" => zeus_tts::AudioFormat::Mp3,
        "opus" => zeus_tts::AudioFormat::Opus,
        _ => zeus_tts::AudioFormat::Wav,
    };

    let provider = zeus_tts::piper::PiperHttpProvider::new(piper_url);
    let stream = provider
        .synthesize_stream(text, voice, speed, audio_format)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("TTS synthesis failed: {e}")))?;

    let content_type = match format {
        "mp3" => "audio/mpeg",
        "opus" => "audio/opus",
        _ => "audio/wav",
    };

    use futures::StreamExt;
    let body_stream = stream.map(|chunk| match chunk {
        Ok(bytes) => Ok::<_, std::io::Error>(bytes),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    });

    let body = axum::body::Body::from_stream(body_stream);
    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], body))
}

pub async fn list_tts_voices(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let provider = params.get("provider").map(|s| s.as_str()).unwrap_or("all");
    let mut voices = vec![];

    if provider == "all" || provider == "openai" {
        voices.push(json!({"provider": "openai", "voice_id": "alloy", "name": "Alloy", "gender": "neutral"}));
        voices.push(json!({"provider": "openai", "voice_id": "echo", "name": "Echo", "gender": "male"}));
        voices.push(json!({"provider": "openai", "voice_id": "fable", "name": "Fable", "gender": "female"}));
        voices.push(json!({"provider": "openai", "voice_id": "onyx", "name": "Onyx", "gender": "male"}));
        voices.push(json!({"provider": "openai", "voice_id": "nova", "name": "Nova", "gender": "female"}));
        voices.push(json!({"provider": "openai", "voice_id": "shimmer", "name": "Shimmer", "gender": "female"}));
    }
    if provider == "all" || provider == "elevenlabs" {
        voices.push(json!({"provider": "elevenlabs", "voice_id": "rachel", "name": "Rachel", "gender": "female"}));
        voices.push(json!({"provider": "elevenlabs", "voice_id": "adam", "name": "Adam", "gender": "male"}));
    }
    if provider == "all" || provider == "edge" {
        voices.push(json!({"provider": "edge", "voice_id": "en-US-AriaNeural", "name": "Aria", "gender": "female"}));
        voices.push(json!({"provider": "edge", "voice_id": "en-US-GuyNeural", "name": "Guy", "gender": "male"}));
    }
    if provider == "all" || provider == "piper" {
        voices.push(json!({"provider": "piper", "voice_id": "en_US-lessac-medium", "name": "Lessac", "gender": "female"}));
        voices.push(json!({"provider": "piper", "voice_id": "en_US-amy-medium", "name": "Amy", "gender": "female"}));
        voices.push(json!({"provider": "piper", "voice_id": "en_US-ryan-medium", "name": "Ryan", "gender": "male"}));
    }
    if provider == "all" || provider == "system" {
        voices.push(json!({"provider": "system", "voice_id": "samantha", "name": "Samantha", "gender": "female", "note": "macOS built-in"}));
        voices.push(json!({"provider": "system", "voice_id": "alex", "name": "Alex", "gender": "male", "note": "macOS built-in"}));
        voices.push(json!({"provider": "system", "voice_id": "daniel", "name": "Daniel", "gender": "male", "note": "macOS built-in"}));
    }

    Json(json!({ "voices": voices, "total": voices.len(), "source": "catalog" }))
}
