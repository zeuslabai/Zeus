//! OpenAI Codex backend — OAuth tokens via Responses API
//!
//! Uses the OpenAI Responses API format (not Chat Completions) with
//! OAuth tokens from ChatGPT browser login.
//!
//! Key differences from standard OpenAI:
//! - Base URL: `https://api.openai.com/v1` (Responses API)
//! - Request format: Responses API (`input` array, not `messages`)
//! - Auth: Bearer token from ChatGPT OAuth (not `sk-` API key)
//! - Token refresh: `POST https://auth.openai.com/oauth/token`

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use zeus_core::{Error, Message, Result, Role, ToolCall, ToolSchema};
use crate::{LlmResponse, StopReason};

// Codex OAuth tokens only work against chatgpt.com/backend-api/codex.
// api.openai.com/v1 rejects them (401 — wrong scopes).
// Requires: instructions field, store=false, stream=true.
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Identity extracted from a Codex JWT access token.
#[derive(Debug, Clone)]
pub struct CodexIdentity {
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub expires_at: Option<u64>,
}

/// Parse identity claims from a Codex JWT access token.
/// The token is a standard JWT with claims at `https://api.openai.com/profile`
/// and `https://api.openai.com/auth`.
pub fn parse_codex_jwt_identity(token: &str) -> Option<CodexIdentity> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 { return None; }

    // Decode payload (second segment)
    let payload = parts[1];
    let padded = match payload.len() % 4 {
        2 => format!("{}==", payload),
        3 => format!("{}=", payload),
        _ => payload.to_string(),
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(padded.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;

    let profile = claims.get("https://api.openai.com/profile");
    let auth = claims.get("https://api.openai.com/auth");

    let email = profile
        .and_then(|p| p.get("email"))
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());

    let account_id = auth
        .and_then(|a| a.get("chatgpt_account_user_id")
            .or_else(|| a.get("chatgpt_user_id"))
            .or_else(|| a.get("user_id")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let user_id = auth
        .and_then(|a| a.get("user_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let expires_at = claims.get("exp")
        .and_then(|v| v.as_u64());

    Some(CodexIdentity { email, account_id, user_id, expires_at })
}

/// Convert Zeus messages to OpenAI Responses API input format.
pub fn to_responses_input(
    messages: &[Message],
    system: Option<&str>,
) -> Vec<Value> {
    let mut input = Vec::new();

    // System prompt as developer role
    if let Some(sys) = system {
        input.push(json!({
            "role": "developer",
            "content": sys
        }));
    }

    for msg in messages {
        match msg.role {
            Role::System => {
                input.push(json!({
                    "role": "developer",
                    "content": msg.content
                }));
            }
            Role::User => {
                input.push(json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": msg.content}]
                }));
            }
            Role::Assistant => {
                // Assistant messages with tool calls
                if !msg.tool_calls.is_empty() {
                    for tc in &msg.tool_calls {
                        input.push(json!({
                            "type": "function_call",
                            "name": tc.name,
                            "arguments": tc.arguments.to_string(),
                            "call_id": tc.id
                        }));
                    }
                }
                if !msg.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": msg.content}]
                    }));
                }
            }
            Role::Tool => {
                for tr in &msg.tool_results {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": tr.call_id,
                        "output": tr.output
                    }));
                }
            }
        }
    }

    input
}

/// Convert Zeus tool schemas to Responses API function tools.
pub fn to_responses_tools(tools: &[ToolSchema]) -> Vec<Value> {
    tools.iter().map(|t| {
        json!({
            "type": "function",
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
            "strict": false
        })
    }).collect()
}

/// Complete a request through the Codex backend.
///
/// The Codex `/responses` endpoint only supports streaming — it rejects
/// `"stream": false` with a `400 "Stream must be set to true"`. We keep the
/// synchronous `complete_codex` signature for callers that want a buffered
/// full response (heartbeat probes, cooking-loop sync fallback, planner
/// calls) by driving `stream_codex` internally: open the SSE stream, drain
/// the chunk channel to completion, then await the background task for the
/// fully-assembled `LlmResponse`. From the caller's perspective nothing
/// changes; from the wire's perspective we're always streaming.
pub async fn complete_codex(
    client: &Client,
    model: &str,
    messages: &[Message],
    tools: &[ToolSchema],
    system: Option<&str>,
    access_token: &str,
) -> Result<LlmResponse> {
    let (mut rx, handle) = stream_codex(client, model, messages, tools, system, access_token).await?;

    // Drain the streaming channel — the background task accumulates the full
    // response internally, and the handle resolves when the SSE stream closes.
    // Discarding the chunks here is intentional: `complete_codex` is the
    // non-streaming API and mustn't emit partial text to the caller.
    while rx.recv().await.is_some() {}

    handle
        .await
        .map_err(|e| Error::Llm(format!("Codex stream task join failed: {}", e)))
}

/// Stream a request through the Codex backend.
pub async fn stream_codex(
    client: &Client,
    model: &str,
    messages: &[Message],
    tools: &[ToolSchema],
    system: Option<&str>,
    access_token: &str,
) -> Result<(mpsc::Receiver<String>, tokio::task::JoinHandle<LlmResponse>)> {
    let input = to_responses_input(messages, system);

    // Codex backend requires: instructions, store=false, stream=true
    let instructions = system.unwrap_or("You are a helpful assistant.");
    let mut body = json!({
        "model": model,
        "input": input,
        "instructions": instructions,
        "store": false,
        "stream": true,
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(to_responses_tools(tools));
    }

    let response = client
        .post(format!("{}/responses", CODEX_BASE_URL))
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Llm(format!("Codex stream request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(Error::Llm(format!("Codex stream API error {}: {}", status, text)));
    }

    let (tx, rx) = mpsc::channel(100);

    let handle = tokio::spawn(async move {
        use futures::StreamExt;

        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut input_tokens: usize = 0;
        let mut output_tokens: usize = 0;
        let mut current_tool_name = String::new();
        let mut current_tool_args = String::new();
        let mut current_tool_id = String::new();

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => continue,
            };

            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process SSE events
            while let Some(newline_pos) = buffer.find("\n\n") {
                let event_block = buffer[..newline_pos].to_string();
                buffer = buffer[newline_pos + 2..].to_string();

                for line in event_block.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" { continue; }

                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            let event_type = event.get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("");

                            match event_type {
                                // Text content delta
                                "response.output_text.delta" => {
                                    if let Some(delta) = event.get("delta")
                                        .and_then(|d| d.as_str())
                                    {
                                        content.push_str(delta);
                                        let _ = tx.send(delta.to_string()).await;
                                    }
                                }
                                // Function call started
                                "response.function_call_arguments.delta" => {
                                    if let Some(delta) = event.get("delta")
                                        .and_then(|d| d.as_str())
                                    {
                                        current_tool_args.push_str(delta);
                                    }
                                }
                                // Output item added (may be function_call)
                                "response.output_item.added" => {
                                    if let Some(item) = event.get("item") {
                                        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                                            current_tool_name = item.get("name")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            current_tool_id = item.get("call_id")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            current_tool_args.clear();
                                        }
                                    }
                                }
                                // Output item done (finalize tool call)
                                "response.output_item.done" => {
                                    if let Some(item) = event.get("item") {
                                        if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                                            let args: Value = serde_json::from_str(&current_tool_args)
                                                .unwrap_or(json!({}));
                                            tool_calls.push(ToolCall {
                                                id: if current_tool_id.is_empty() {
                                                    format!("call_{}", uuid::Uuid::new_v4())
                                                } else {
                                                    current_tool_id.clone()
                                                },
                                                name: current_tool_name.clone(),
                                                arguments: args,
                                            });
                                            current_tool_name.clear();
                                            current_tool_args.clear();
                                            current_tool_id.clear();
                                        }
                                    }
                                }
                                // Response completed — extract usage
                                "response.completed" => {
                                    if let Some(resp) = event.get("response") {
                                        if let Some(usage) = resp.get("usage") {
                                            input_tokens = usage.get("input_tokens")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as usize;
                                            output_tokens = usage.get("output_tokens")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0) as usize;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        let stop_reason = if !tool_calls.is_empty() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        LlmResponse {
            content,
            tool_calls,
            stop_reason,
            input_tokens,
            output_tokens,
            cached_tokens: 0,
        }
    });

    Ok((rx, handle))
}

/// Parse a non-streaming Responses API output into LlmResponse.
fn parse_responses_output(response: &Value) -> Result<LlmResponse> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(msg_content) = item.get("content").and_then(|c| c.as_array()) {
                        for block in msg_content {
                            if block.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    content.push_str(text);
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let call_id = item.get("call_id").and_then(|n| n.as_str()).unwrap_or("").to_string();
                    let args_str = item.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    tool_calls.push(ToolCall {
                        id: if call_id.is_empty() { format!("call_{}", uuid::Uuid::new_v4()) } else { call_id },
                        name,
                        arguments: args,
                    });
                }
                _ => {}
            }
        }
    }

    let usage = response.get("usage");
    let input_tokens = usage.and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let output_tokens = usage.and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64()).unwrap_or(0) as usize;

    let stop_reason = if !tool_calls.is_empty() {
        StopReason::ToolUse
    } else {
        StopReason::EndTurn
    };

    Ok(LlmResponse {
        content,
        tool_calls,
        stop_reason,
        input_tokens,
        output_tokens,
        cached_tokens: 0,
    })
}

/// Refresh a Codex OAuth access token using the refresh token.
/// Refresh a Codex OAuth access token using the refresh token.
/// If refresh fails but `current_access_token` is provided and non-empty,
/// returns the stale token as a lenient fallback (matches OpenClaw behavior).
pub async fn refresh_codex_token(
    client: &Client,
    refresh_token: &str,
) -> Result<(String, Option<String>)> {
    refresh_codex_token_lenient(client, refresh_token, None).await
}

/// Lenient refresh — falls back to stale token on transient errors.
pub async fn refresh_codex_token_lenient(
    client: &Client,
    refresh_token: &str,
    current_access_token: Option<&str>,
) -> Result<(String, Option<String>)> {
    let resp = match client
        .post(CODEX_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CODEX_CLIENT_ID),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            // Network error — fall back to stale token if available
            if let Some(stale) = current_access_token.filter(|t| !t.is_empty()) {
                warn!("Codex token refresh failed ({}), using stale token", e);
                return Ok((stale.to_string(), None));
            }
            return Err(Error::Llm(format!("Codex token refresh failed: {}", e)));
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        // Lenient fallback — use stale token if refresh fails
        if let Some(stale) = current_access_token.filter(|t| !t.is_empty()) {
            warn!("Codex token refresh failed ({}), using stale token", status);
            return Ok((stale.to_string(), None));
        }
        return Err(Error::Llm(format!("Codex token refresh failed ({}): {}", status, text)));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: Option<String>,
    }

    let tokens: TokenResponse = resp.json().await
        .map_err(|e| Error::Llm(format!("Failed to parse Codex refresh response: {}", e)))?;

    Ok((tokens.access_token, tokens.refresh_token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[test]
    fn test_parse_codex_jwt_identity() {
        use base64::Engine;
        // Build a fake JWT with OpenAI claims
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"exp":1775740827,"https://api.openai.com/profile":{"email":"test@example.com"},"https://api.openai.com/auth":{"chatgpt_account_user_id":"user-abc123","user_id":"user-xyz"}}"#);
        let token = format!("{}.{}.fake_signature", header, payload);

        let identity = parse_codex_jwt_identity(&token).unwrap();
        assert_eq!(identity.email.as_deref(), Some("test@example.com"));
        assert_eq!(identity.account_id.as_deref(), Some("user-abc123"));
        assert_eq!(identity.user_id.as_deref(), Some("user-xyz"));
        assert_eq!(identity.expires_at, Some(1775740827));
    }

    #[test]
    fn test_parse_codex_jwt_invalid_token() {
        assert!(parse_codex_jwt_identity("not-a-jwt").is_none());
        assert!(parse_codex_jwt_identity("a.b").is_none());
        assert!(parse_codex_jwt_identity("").is_none());
    }

    #[test]
    fn test_to_responses_input_basic() {
        let msgs = vec![
            Message::user("Hello"),
            Message::assistant("Hi there"),
        ];
        let input = to_responses_input(&msgs, Some("You are helpful"));
        assert_eq!(input.len(), 3); // developer + user + assistant
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[1]["role"], "user");
        assert_eq!(input[1]["content"][0]["type"], "input_text");
        assert_eq!(input[2]["role"], "assistant");
    }

    #[test]
    fn test_to_responses_tools() {
        let tools = vec![
            ToolSchema::new("test_tool", "A test tool")
                .with_param("arg1", "string", "An argument", true),
        ];
        let converted = to_responses_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["name"], "test_tool");
    }

    #[test]
    fn test_parse_responses_output_text() {
        let response = json!({
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "Hello world"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });
        let result = parse_responses_output(&response).unwrap();
        assert_eq!(result.content, "Hello world");
        assert_eq!(result.input_tokens, 10);
        assert_eq!(result.output_tokens, 5);
    }

    #[test]
    fn test_parse_responses_output_tool_call() {
        let response = json!({
            "output": [{
                "type": "function_call",
                "name": "read_file",
                "call_id": "call_123",
                "arguments": "{\"path\":\"/tmp/test\"}"
            }],
            "usage": {"input_tokens": 15, "output_tokens": 8}
        });
        let result = parse_responses_output(&response).unwrap();
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "read_file");
        assert_eq!(result.tool_calls[0].id, "call_123");
    }
}
