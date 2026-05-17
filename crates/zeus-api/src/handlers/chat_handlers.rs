//! Chat + OpenAI-compatible handler endpoints

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info};

use crate::SharedState;
use zeus_session::Session;
use zeus_llm::LlmClient;
use super::{model_tier_cost, openai_error};

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub response: String,
    pub session_id: String,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Serialize)]
pub struct ToolCallInfo {
    pub name: String,
    pub success: bool,
    pub output: String,
}


#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    /// Source identifier (e.g., "telegram", "discord", "slack")
    #[serde(default)]
    pub source: Option<String>,
    /// Message content
    pub message: String,
    /// Optional sender identifier
    #[serde(default)]
    pub sender: Option<String>,
    /// Optional channel/chat identifier
    #[serde(default)]
    pub channel: Option<String>,
    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub received: bool,
    pub id: String,
    pub processed: bool,
}


#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<usize>,
}

// Phase 2 Request/Response Types

#[derive(Debug, Deserialize)]
pub struct ImageGenRequest {
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Override the configured provider for this request
    #[serde(default)]
    pub provider: Option<String>,
    /// Override the model for this request
    #[serde(default)]
    pub model: Option<String>,
    /// Number of images to generate (default 1)
    #[serde(default)]
    pub n: Option<u32>,
    /// Inference steps (local providers only)
    #[serde(default)]
    pub steps: Option<u32>,
    /// Random seed for reproducibility
    #[serde(default)]
    pub seed: Option<i64>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)] // Documents the response shape; we return Json<Value> directly
pub struct ImageGenResponse {
    pub image_id: String,
    pub image_base64: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
}

// Health & Status

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "server": "zeus-api",
        "version": crate::VERSION
    }))
}

/// Detailed health check with subsystem status.
/// Reports which subsystems initialized successfully vs failed.
pub async fn health_detailed(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    Json(json!({
        "status": "ok",
        "version": crate::VERSION,
        "subsystems": {
            "mnemosyne": state.mnemosyne.is_some(),
            "nous": state.nous.is_some(),
            "agent": state.default_agent.is_some(),
            "tool_executor": state.tool_executor.is_some(),
            "channels": true, // ChannelManager always initialized (Arc, not Option)
        }
    }))
}

pub async fn status(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let (provider, model) = state.config.parse_model();

    // Count sessions
    let sessions_count = Session::list(&state.config.sessions)
        .await
        .map(|s| s.len())
        .unwrap_or(0);

    // Determine auth method
    let auth_method = if state.config.auth.use_oauth {
        "oauth"
    } else {
        let env_key = match provider {
            zeus_core::Provider::Anthropic => "ANTHROPIC_API_KEY",
            zeus_core::Provider::OpenAI => "OPENAI_API_KEY",
            zeus_core::Provider::OpenRouter => "OPENROUTER_API_KEY",
            zeus_core::Provider::Google => "GOOGLE_API_KEY",
            zeus_core::Provider::Groq => "GROQ_API_KEY",
            zeus_core::Provider::Mistral => "MISTRAL_API_KEY",
            zeus_core::Provider::Together => "TOGETHER_API_KEY",
            zeus_core::Provider::Fireworks => "FIREWORKS_API_KEY",
            zeus_core::Provider::Azure => "AZURE_OPENAI_API_KEY",
            zeus_core::Provider::Bedrock => "AWS_ACCESS_KEY_ID",
            zeus_core::Provider::Ollama => "",
            zeus_core::Provider::DeepSeek => "DEEPSEEK_API_KEY",
            zeus_core::Provider::XAI => "XAI_API_KEY",
            zeus_core::Provider::Cerebras => "CEREBRAS_API_KEY",
            zeus_core::Provider::Moonshot => "MOONSHOT_API_KEY",
            zeus_core::Provider::Zai => "ZAI_API_KEY",
            zeus_core::Provider::Qwen => "QWEN_API_KEY",
            zeus_core::Provider::Minimax => "",
            zeus_core::Provider::XiaomiMimo => "XIAOMIMIMO_API_KEY",
            zeus_core::Provider::GoogleGeminiCli => "",
        };
        if env_key.is_empty() || std::env::var(env_key).is_ok() {
            "api_key"
        } else {
            "none"
        }
    };

    let gateway_url = state
        .config
        .gateway
        .as_ref()
        .map(|g| g.public_url.clone())
        .unwrap_or_default();

    let agent_name = state.config.agent.as_ref().and_then(|a| a.name.as_deref())
        .or_else(|| state.config.name.as_deref())
        .or_else(|| state.config.network.as_ref().and_then(|n| n.agent_name.as_deref()))
        .or_else(|| state.config.agents.first().map(|a| a.id.as_str()))
        .unwrap_or("zeus")
        .to_string();

    Json(json!({
        "status": "ok",
        "provider": format!("{:?}", provider),
        "model": model,
        "agent_name": agent_name,
        "workspace": state.workspace.root().display().to_string(),
        "tools": state.tools.schemas().len(),
        "version": crate::VERSION,
        "auth_method": auth_method,
        "sessions_count": sessions_count,
        "gateway_url": gateway_url
    }))
}

// Chat

pub async fn chat(
    State(state): State<SharedState>,
    Json(req): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, (StatusCode, String)> {
    info!("Chat request: {} chars", req.message.len());

    let state = state.read().await;

    // Use the agent's persistent session (unified across all channels).
    // If session_id provided, load that specific session.
    // Otherwise use the gateway's main session so TUI shares context with Discord/etc.
    let mut session = if let Some(id) = &req.session_id {
        // resume_or_create: load existing session or create new one with this ID
        Session::resume_or_create(&state.config.sessions, id).await
    } else if let Some(ref agent) = state.default_agent {
        // Load the gateway agent's session by ID — TUI shares context with Discord
        let session_id = {
            let agent_guard = agent.read().await;
            agent_guard.session().id.clone()
        };
        Session::resume_or_create(&state.config.sessions, &session_id).await
    } else {
        // Fallback: create ephemeral session (no agent running)
        let s = Session::new(&state.config.sessions);
        s.init()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        s
    };

    let session_id = session.id.clone();

    // Use the full agent pipeline (with tools, cooking loop, workspace context)
    // Same path as Discord/Telegram — full tool access, not just text completion
    if let Some(ref agent) = state.default_agent {
        // Prefer inbox path (non-blocking, serialized by consumer) over Mutex path.
        let response = if let Some(inbox) = state.agent_inbox.as_ref() {
            inbox.send_and_wait(
                req.message.clone(),
                Some(zeus_core::ChannelSource {
                    channel_type: "tui".to_string(),
                    channel_id: None,
                    channel_name: Some("TUI".to_string()),
                    sender_name: Some("You".to_string()),
                    sender_id: None,
                }),
                vec![],
                1800, // 30 min — complex tasks need time
                true, // Use cooking loop — same path as Discord/Telegram
                None, // #66 Cut 3: TUI/API path — no mention classification
            ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        } else {
            // Fallback: S98 Mutex path when inbox is not wired
            let _run_guard = state.agent_run_lock.lock().await;
            let mut agent_guard = agent.write().await;
            agent_guard.run(&req.message).await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        };

        Ok(Json(ChatResponse {
            response,
            session_id,
            tool_calls: vec![],
        }))
    } else {
        // Fallback: no agent running — use simple LLM call
        let llm = LlmClient::from_config(&state.config)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let user_msg = zeus_core::Message::user(&req.message)
            .with_channel_source(zeus_core::ChannelSource {
                channel_type: "tui".to_string(),
                channel_id: None,
                channel_name: Some("TUI".to_string()),
                sender_name: Some("You".to_string()),
                sender_id: None,
            });
        session
            .add(user_msg)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let system_prompt = state
            .workspace
            .get_context()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let tool_schemas = state.tools.schemas();

        let response = llm
            .complete(&session.messages, &tool_schemas, Some(&system_prompt))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let assistant_msg =
            zeus_core::Message::assistant(&response.content).with_tool_calls(response.tool_calls);
        session
            .add(assistant_msg)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        Ok(Json(ChatResponse {
            response: response.content,
            session_id,
            tool_calls: vec![],
        }))
    }
}

// OpenAI-Compatible API

/// OpenAI ChatCompletion request format
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct OpenAIChatRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<OpenAIMessage>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

/// POST /v1/chat/completions — OpenAI-compatible chat endpoint
pub async fn openai_chat_completions(
    State(state): State<SharedState>,
    Json(req): Json<OpenAIChatRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let stream = req.stream.unwrap_or(false);

    if stream {
        openai_chat_stream(state, req).await
    } else {
        openai_chat_non_stream(state, req)
            .await
            .map(|json| json.into_response())
    }
}

/// Non-streaming OpenAI-compatible chat completion
async fn openai_chat_non_stream(
    state: SharedState,
    req: OpenAIChatRequest,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Extract the last user message as the prompt for agent.run()
    let last_user_msg = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let created = chrono::Utc::now().timestamp();
    let model_name = req
        .model
        .as_deref()
        .unwrap_or(&state.config.model)
        .to_string();
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());

    // Route through agent.run() when an agent is running — same as chat() handler.
    // This gives full tool access + cooking loop to OpenAI-compatible clients.
    if let Some(ref agent) = state.default_agent {
        // Prefer inbox path; fall back to Mutex if inbox not wired.
        let response_text = if let Some(inbox) = state.agent_inbox.as_ref() {
            inbox.send_and_wait(
                last_user_msg.clone(),
                None,
                vec![],
                1800, // 30 min — complex tasks need time
                true, // Use cooking loop — same path as Discord/Telegram
                None, // #66 Cut 3: OpenAI-compat path — no mention classification
            ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, openai_error(&e)))?
        } else {
            // S98: Serialize agent.run() calls
            let _run_guard = state.agent_run_lock.lock().await;
            let mut agent_guard = agent.write().await;
            agent_guard
                .run(&last_user_msg)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, openai_error(&e.to_string())))?
        };

        // Record LLM spend in economy
        {
            let cost = model_tier_cost(&model_name);
            if let Err(e) = state.ledger.spend(
                "default",
                cost,
                zeus_economy::TransactionReason::LlmCall,
                format!("completion: {}", model_name),
            ) {
                debug!("Economy spend failed (non-fatal): {e}");
            }
        }

        return Ok(Json(json!({
            "id": completion_id,
            "object": "chat.completion",
            "created": created,
            "model": model_name,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": response_text,
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": 0,
                "total_tokens": 0,
            }
        })));
    }

    // Fallback: no agent running — use bare LLM call with full tool schemas
    let llm = LlmClient::from_config(&state.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            openai_error(&e.to_string()),
        )
    })?;

    // Convert OpenAI messages to Zeus messages
    let zeus_messages: Vec<zeus_core::Message> = req
        .messages
        .iter()
        .map(|m| match m.role.as_str() {
            "system" => zeus_core::Message::system(&m.content),
            "assistant" => zeus_core::Message::assistant(&m.content),
            _ => zeus_core::Message::user(&m.content),
        })
        .collect();

    // Get workspace context as system prompt
    let system_prompt = state.workspace.get_context().await.unwrap_or_default();

    // Pass full tool schemas (not empty slice) so the LLM at least sees tools
    let tool_schemas = state.tools.schemas();

    let response = llm
        .complete(&zeus_messages, &tool_schemas, Some(&system_prompt))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                openai_error(&e.to_string()),
            )
        })?;

    // Record LLM spend in economy
    {
        let cost = model_tier_cost(&model_name);
        if let Err(e) = state.ledger.spend(
            "default",
            cost,
            zeus_economy::TransactionReason::LlmCall,
            format!("completion: {}", model_name),
        ) {
            debug!("Economy spend failed (non-fatal): {e}");
        }
    }

    let finish_reason = match response.stop_reason {
        zeus_llm::StopReason::EndTurn => "stop",
        zeus_llm::StopReason::MaxTokens => "length",
        zeus_llm::StopReason::ToolUse => "tool_calls",
        zeus_llm::StopReason::Error => "stop",
    };

    Ok(Json(json!({
        "id": completion_id,
        "object": "chat.completion",
        "created": created,
        "model": model_name,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": response.content,
            },
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": response.input_tokens,
            "completion_tokens": response.output_tokens,
            "total_tokens": response.input_tokens + response.output_tokens,
        }
    })))
}

/// Streaming OpenAI-compatible chat completion (SSE)
///
/// Routes through the agent inbox (full tool execution + cooking loop) instead
/// of a bare LlmClient::stream(). The agent processes the message completely
/// (including tool calls), then we stream the final response back as SSE chunks.
/// This trades per-token streaming for correctness — tool calls actually execute.
async fn openai_chat_stream(
    state: SharedState,
    req: OpenAIChatRequest,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::response::sse::{Event, Sse};
    use std::convert::Infallible;

    let state_read = state.read().await;

    let model_name = req
        .model
        .clone()
        .unwrap_or_else(|| state_read.config.model.clone());
    let completion_id = format!("chatcmpl-{}", uuid::Uuid::new_v4().simple());
    let created = chrono::Utc::now().timestamp();

    // Extract the last user message as the prompt
    let last_user_msg = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    // ── Agent path: route through inbox for full tool execution ──────────
    if let Some(inbox) = state_read.agent_inbox.as_ref().cloned() {
        // Record LLM spend
        let cost = model_tier_cost(&model_name);
        if let Err(e) = state_read.ledger.spend(
            "default",
            cost,
            zeus_economy::TransactionReason::LlmCall,
            format!("stream: {}", &model_name),
        ) {
            debug!("Economy spend failed (non-fatal): {e}");
        }

        // Capture broadcast handle before dropping the lock
        let chat_broadcast = state_read.chat_broadcast.clone();

        // Drop the read lock before awaiting the agent
        drop(state_read);

        // Send through inbox with cooking loop — TUI uses the same path as
        // Discord/Telegram: full cooking loop, session persistence, auto-compaction.
        // Previously false (agent.run only), which caused orphaned tool_calls on
        // Kimi K2.6 and session accumulation issues on all providers.
        let mut stream_rx = inbox.send_and_stream(
            last_user_msg,
            Some(zeus_core::ChannelSource {
                channel_type: "tui".to_string(),
                channel_id: None,
                channel_name: Some("TUI".to_string()),
                sender_name: Some("You".to_string()),
                sender_id: None,
            }),
            vec![],
            1800, // 30 min — complex tasks need time
            true,
            None, // #66 Cut 3: streaming TUI path — no mention classification
        ).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, openai_error(&e)))?;

        let stream = async_stream::stream! {
            // Wait for the agent to finish (Done chunk contains the full response)
            // 2.3: track thinking block state across chunks
            let mut in_think = false;
            while let Some(chunk) = stream_rx.recv().await {
                match chunk {
                    zeus_core::inbox::StreamChunk::Token(token) => {
                        // 2.3: parse <think>...</think> tags injected by stream_ollama for
                        // thinking models. Route thinking segments to delta.thinking (→
                        // SseEvent::Thinking in TUI) and content segments to delta.content.
                        let mut remaining = token.as_str();
                        while !remaining.is_empty() {
                            if let Some(rest) = remaining.strip_prefix("<think>") {
                                in_think = true;
                                remaining = rest;
                            } else if let Some(rest) = remaining.strip_prefix("</think>") {
                                in_think = false;
                                remaining = rest;
                            } else {
                                let end = remaining.find('<').unwrap_or(remaining.len());
                                let segment = &remaining[..end];
                                if !segment.is_empty() {
                                    let data = if in_think {
                                        json!({
                                            "id": &completion_id,
                                            "object": "chat.completion.chunk",
                                            "created": created,
                                            "model": &model_name,
                                            "choices": [{"index": 0, "delta": {"thinking": segment}, "finish_reason": null}]
                                        })
                                    } else {
                                        json!({
                                            "id": &completion_id,
                                            "object": "chat.completion.chunk",
                                            "created": created,
                                            "model": &model_name,
                                            "choices": [{"index": 0, "delta": {"content": segment}, "finish_reason": null}]
                                        })
                                    };
                                    yield Ok::<_, Infallible>(Event::default().data(data.to_string()));

                                    // Broadcast token to TUI / WebSocket subscribers
                                    chat_broadcast.send(crate::chat_broadcast::StreamToken {
                                        text: segment.to_string(),
                                        is_thinking: in_think,
                                        tab: Some("chat".to_string()),
                                    });
                                }
                                remaining = &remaining[end..];
                            }
                        }
                    }
                    zeus_core::inbox::StreamChunk::Thinking(text) => {
                        let data = json!({ "event": "thinking", "text": text });
                        yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                    }
                    zeus_core::inbox::StreamChunk::ToolStart { name, input } => {
                        let data = json!({
                            "event": "tool_start",
                            "tool": name,
                            "input": input,
                        });
                        yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                    }
                    zeus_core::inbox::StreamChunk::ToolEnd { name, output } => {
                        let data = json!({
                            "event": "tool_end",
                            "tool": name,
                            "output": output,
                        });
                        yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                    }
                    zeus_core::inbox::StreamChunk::Iter(n) => {
                        let data = json!({
                            "event": "iter",
                            "n": n,
                        });
                        yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                    }
                    zeus_core::inbox::StreamChunk::Done(result) => {
                        // Tokens were already streamed via StreamChunk::Token events.
                        // Only send error text if the result was an error (no tokens were sent).
                        if let Err(e) = &result {
                            let data = json!({
                                "id": &completion_id,
                                "object": "chat.completion.chunk",
                                "created": created,
                                "model": &model_name,
                                "choices": [{
                                    "index": 0,
                                    "delta": { "content": format!("Error: {}", e) },
                                    "finish_reason": null,
                                }]
                            });
                            yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                        }

                        // Send final chunk with finish_reason
                        let final_data = json!({
                            "id": &completion_id,
                            "object": "chat.completion.chunk",
                            "created": created,
                            "model": &model_name,
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": "stop",
                            }]
                        });
                        yield Ok::<_, Infallible>(Event::default().data(final_data.to_string()));
                        break;
                    }
                }
            }

            // Send [DONE] marker
            yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
        };

        return Ok(Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response());
    }

    // ── Fallback: no agent running — bare LLM streaming (no tool execution) ─
    let llm = LlmClient::from_config(&state_read.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            openai_error(&e.to_string()),
        )
    })?;

    let zeus_messages: Vec<zeus_core::Message> = req
        .messages
        .iter()
        .map(|m| match m.role.as_str() {
            "system" => zeus_core::Message::system(&m.content),
            "assistant" => zeus_core::Message::assistant(&m.content),
            _ => zeus_core::Message::user(&m.content),
        })
        .collect();

    let system_prompt = state_read.workspace.get_context().await.unwrap_or_default();

    let tool_schemas = state_read.tools.schemas();

    let (mut rx, handle) = llm
        .stream(&zeus_messages, &tool_schemas, Some(&system_prompt))
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                openai_error(&e.to_string()),
            )
        })?;

    // Record LLM spend in economy (before dropping lock)
    let cost = model_tier_cost(&model_name);
    if let Err(e) = state_read.ledger.spend(
        "default",
        cost,
        zeus_economy::TransactionReason::LlmCall,
        format!("stream: {}", &model_name),
    ) {
        debug!("Economy spend failed (non-fatal): {e}");
    }

    // Drop the read lock before spawning the stream
    drop(state_read);

    let stream = async_stream::stream! {
        // Stream text chunks as SSE events
        // 2.3: thinking block state for <think> tag routing
        let mut in_think = false;
        while let Some(chunk) = rx.recv().await {
            let mut remaining = chunk.as_str();
            while !remaining.is_empty() {
                if let Some(rest) = remaining.strip_prefix("<think>") {
                    in_think = true;
                    remaining = rest;
                } else if let Some(rest) = remaining.strip_prefix("</think>") {
                    in_think = false;
                    remaining = rest;
                } else {
                    let end = remaining.find('<').unwrap_or(remaining.len());
                    let segment = &remaining[..end];
                    if !segment.is_empty() {
                        let data = if in_think {
                            json!({
                                "id": &completion_id,
                                "object": "chat.completion.chunk",
                                "created": created,
                                "model": &model_name,
                                "choices": [{"index": 0, "delta": {"thinking": segment}, "finish_reason": null}]
                            })
                        } else {
                            json!({
                                "id": &completion_id,
                                "object": "chat.completion.chunk",
                                "created": created,
                                "model": &model_name,
                                "choices": [{"index": 0, "delta": {"content": segment}, "finish_reason": null}]
                            })
                        };
                        yield Ok::<_, Infallible>(Event::default().data(data.to_string()));
                    }
                    remaining = &remaining[end..];
                }
            }
        }

        // Wait for the final response to get finish_reason
        let finish_reason = match handle.await {
            Ok(resp) => match resp.stop_reason {
                zeus_llm::StopReason::EndTurn => "stop",
                zeus_llm::StopReason::MaxTokens => "length",
                zeus_llm::StopReason::ToolUse => "tool_calls",
                zeus_llm::StopReason::Error => "stop",
            },
            Err(_) => "stop",
        };

        // Send final chunk with finish_reason
        let final_data = json!({
            "id": &completion_id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": &model_name,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": finish_reason,
            }]
        });
        yield Ok::<_, Infallible>(Event::default().data(final_data.to_string()));

        // Send [DONE] marker
        yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
    };

    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response())
}

/// GET /v1/models — OpenAI-compatible model listing
pub async fn openai_list_models(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let model = &state.config.model;

    Json(json!({
        "object": "list",
        "data": [{
            "id": model,
            "object": "model",
            "created": 0,
            "owned_by": "zeus",
        }]
    }))
}
