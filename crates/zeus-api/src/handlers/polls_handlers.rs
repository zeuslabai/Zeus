use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::SharedState;

#[derive(serde::Deserialize)]
pub struct SendPollRequest {
    pub chat_id: String,
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub is_anonymous: bool,
    #[serde(default)]
    pub allows_multiple_answers: bool,
    pub correct_option_id: Option<usize>,
    pub explanation: Option<String>,
}

/// POST /v1/channels/:id/poll — Send a Telegram poll
pub async fn send_poll(
    State(state): State<SharedState>,
    Path(channel_id): Path<String>,
    Json(req): Json<SendPollRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&channel_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Channel not found".to_string()))?;

    if channel.channel_type != crate::channels::ChannelType::Telegram {
        return Err((
            StatusCode::BAD_REQUEST,
            "Poll support is only available for Telegram channels".to_string(),
        ));
    }

    use zeus_channels::ChannelSource;
    use zeus_channels::telegram::{TelegramAdapter, TelegramConfig, TelegramPoll};

    let bot_token = channel
        .config
        .get("bot_token")
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Telegram polls require bot_token in channel config".to_string(),
            )
        })?
        .clone();

    let api_id = channel
        .config
        .get("api_id")
        .and_then(|s: &String| s.parse::<i32>().ok())
        .unwrap_or(0);

    let api_hash = channel.config.get("api_hash").cloned().unwrap_or_default();

    let tg_config = TelegramConfig {
        api_id,
        api_hash,
        bot_token: Some(bot_token),
        phone: None,
        session_path: None,
        allow_bots: None,
    };

    let adapter = TelegramAdapter::new(tg_config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let poll = TelegramPoll {
        question: req.question.clone(),
        options: req.options.clone(),
        is_anonymous: req.is_anonymous,
        allows_multiple_answers: req.allows_multiple_answers,
        correct_option_id: req.correct_option_id,
        explanation: req.explanation.clone(),
    };

    let source = ChannelSource::with_chat("telegram", &req.chat_id, &req.chat_id);

    let message_id = adapter
        .send_poll(&source, &poll)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "message_id": message_id,
        "chat_id": req.chat_id,
        "question": req.question,
    })))
}

/// DELETE /v1/channels/:id/poll/:message_id — Stop a Telegram poll
pub async fn stop_poll(
    State(state): State<SharedState>,
    Path((channel_id, message_id_str)): Path<(String, String)>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let channel = state
        .channel_store
        .get(&channel_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Channel not found".to_string()))?;

    if channel.channel_type != crate::channels::ChannelType::Telegram {
        return Err((
            StatusCode::BAD_REQUEST,
            "Poll support is only available for Telegram channels".to_string(),
        ));
    }

    let chat_id = params.get("chat_id").ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "chat_id query parameter required".to_string(),
        )
    })?;

    let message_id: i64 = message_id_str
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid message_id".to_string()))?;

    use zeus_channels::telegram::{TelegramAdapter, TelegramConfig};

    let bot_token = channel
        .config
        .get("bot_token")
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Telegram polls require bot_token in channel config".to_string(),
            )
        })?
        .clone();

    let api_id = channel
        .config
        .get("api_id")
        .and_then(|s: &String| s.parse::<i32>().ok())
        .unwrap_or(0);

    let api_hash = channel.config.get("api_hash").cloned().unwrap_or_default();

    let tg_config = TelegramConfig {
        api_id,
        api_hash,
        bot_token: Some(bot_token),
        phone: None,
        session_path: None,
        allow_bots: None,
    };

    let adapter = TelegramAdapter::new(tg_config)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result = adapter
        .stop_poll(chat_id, message_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "poll": {
            "poll_id": result.poll_id,
            "question": result.question,
            "options": result.options,
            "total_voter_count": result.total_voter_count,
            "is_closed": result.is_closed,
            "is_anonymous": result.is_anonymous,
            "allows_multiple_answers": result.allows_multiple_answers,
        }
    })))
}
