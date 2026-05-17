#![allow(dead_code)]
//! API client — connects TUI v2 to the Zeus gateway
//!
//! All API calls are async via reqwest. Results update the App state.
//! No rendering code here — pure data fetching.

use serde::{Deserialize, Serialize};

/// Typed SSE events emitted by `chat_stream`.
///
/// The gateway may send standard OpenAI token deltas plus Zeus-specific
/// extension events for tool calls, iteration boundaries, and thinking.
#[derive(Debug, Clone)]
pub enum SseEvent {
    /// A text token chunk from the assistant reply.
    Token(String),
    /// A tool call is starting (Layer 2 — display in TUI).
    ToolStart { name: String, input: String },
    /// A tool call has completed.
    ToolEnd { name: String, output: String },
    /// Iteration boundary — agent is starting iteration N.
    Iter(u32),
    /// Thinking/reasoning text snippet (extended thinking or ThinkingDelta).
    Thinking(String),
    /// Token usage at end of turn.
    Usage { input: usize, output: usize },
}

/// Gateway API client
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Deserialize, Default)]
pub struct StatusResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub tools: usize,
    #[serde(default)]
    pub sessions_count: usize,
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub auth_method: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub gateway_url: String,
    /// Agent's configured name (from config.name / onboarding)
    #[serde(default)]
    pub agent_name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct StatsResponse {
    #[serde(default)]
    pub sessions_count: usize,
    #[serde(default)]
    pub tools_count: usize,
    #[serde(default)]
    pub memory_files: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    /// "local" for gateway agents, "channel" for Discord-discovered agents (S93)
    #[serde(default, rename = "type")]
    pub agent_type: String,
    #[serde(default)]
    pub health_score: f32,
    #[serde(default)]
    pub load_pct: f32,
    #[serde(default)]
    pub last_heartbeat: String,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub current_task: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChannelResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonRoomResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub participant_count: usize,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonMissionResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub agent_count: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PantheonMessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub message_type: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub channel_source: Option<SessionChannelSource>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SessionChannelSource {
    #[serde(default)]
    pub channel_type: String,
    #[serde(default)]
    pub sender_name: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub session_id: String,
}

impl ApiClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(600)) // cooking loop can take 5+ min
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn status(&self) -> Result<StatusResponse, String> {
        self.get("/v1/status").await
    }

    pub async fn stats(&self) -> Result<StatsResponse, String> {
        self.get("/v1/stats").await
    }

    pub async fn agents(&self) -> Result<Vec<AgentResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] agents: Vec<AgentResponse> }
        let resp: Resp = self.get("/v1/network/agents").await?;
        Ok(resp.agents)
    }

    pub async fn channels(&self) -> Result<Vec<ChannelResponse>, String> {
        #[derive(Deserialize)]
        struct Resp { #[serde(default)] channels: Vec<ChannelResponse> }
        let resp: Resp = self.get("/v1/channels").await?;
        Ok(resp.channels)
    }

    pub async fn chat(&self, message: &str, session_id: Option<&str>) -> Result<ChatResponse, String> {
        let req = ChatRequest {
            message: message.to_string(),
            session_id: session_id.map(|s| s.to_string()),
        };
        self.post("/v1/chat", &req).await
    }

    /// Stream a chat response token-by-token via OpenAI-compatible SSE.
    ///
    /// Uses `bytes_stream()` for true incremental delivery — each chunk is
    /// processed as it arrives from the gateway, so `on_token` fires in
    /// real-time rather than after the full response has buffered.
    ///
    /// The `on_event` callback receives typed `SseEvent` values so callers can
    /// display tool calls, iteration counts, and thinking snippets in real-time.
    pub async fn chat_stream<F>(&self, message: &str, mut on_event: F) -> Result<String, String>
    where
        F: FnMut(SseEvent),
    {
        use futures_util::StreamExt;

        #[derive(serde::Serialize)]
        struct OaiReq<'a> {
            model: &'a str,
            messages: Vec<OaiMsg<'a>>,
            stream: bool,
        }
        #[derive(serde::Serialize)]
        struct OaiMsg<'a> { role: &'a str, content: &'a str }

        let req = OaiReq {
            model: "default",
            messages: vec![OaiMsg { role: "user", content: message }],
            stream: true,
        };

        let resp = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("Stream request failed: {e}"))?;

        // Check HTTP status before streaming — return error body for 4xx/5xx
        // instead of streaming garbage that produces blank messages in the TUI.
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_else(|_| "<no body>".to_string());
            return Err(format!("HTTP {}: {}", status, body.trim()));
        }

        let mut stream = resp.bytes_stream();
        let mut full = String::new();
        // SSE lines can span multiple HTTP chunks; we keep a partial-line buffer.
        let mut buf = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream read failed: {e}"))?;
            // Append raw bytes as UTF-8 (gateway always sends UTF-8)
            let text = String::from_utf8_lossy(&chunk);
            buf.push_str(&text);

            // Process all complete lines in the buffer
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim_end_matches('\r').to_string();
                buf = buf[newline_pos + 1..].to_string();

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(full);
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        // ── Standard OpenAI token delta ──────────────────────
                        if let Some(token) = v["choices"][0]["delta"]["content"].as_str() {
                            full.push_str(token);
                            on_event(SseEvent::Token(token.to_string()));
                            continue;
                        }

                        // ── Zeus-specific extensions ─────────────────────────
                        // Tool call start: {"event":"tool_start","tool":"Bash","input":"ls"}
                        if v["event"] == "tool_start" {
                            let name = v["tool"].as_str().unwrap_or("tool").to_string();
                            let input = v["input"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::ToolStart { name, input });
                            continue;
                        }

                        // Tool call result: {"event":"tool_end","tool":"Bash","output":"..."}
                        if v["event"] == "tool_end" {
                            let name = v["tool"].as_str().unwrap_or("tool").to_string();
                            let output = v["output"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::ToolEnd { name, output });
                            continue;
                        }

                        // Iteration boundary: {"event":"iter","n":3}
                        if v["event"] == "iter" {
                            let n = v["n"].as_u64().unwrap_or(0) as u32;
                            on_event(SseEvent::Iter(n));
                            continue;
                        }

                        // Thinking delta: {"event":"thinking","text":"..."}
                        if v["event"] == "thinking" {
                            let text = v["text"].as_str().unwrap_or("").to_string();
                            on_event(SseEvent::Thinking(text));
                            continue;
                        }

                        // Anthropic-style thinking in delta: {"choices":[{"delta":{"thinking":"..."}}]}
                        if let Some(thinking) = v["choices"][0]["delta"]["thinking"].as_str() {
                            on_event(SseEvent::Thinking(thinking.to_string()));
                        }

                        // Usage event: {"event":"usage","input_tokens":N,"output_tokens":N}
                        if v["event"] == "usage" {
                            let input = v["input_tokens"].as_u64().unwrap_or(0) as usize;
                            let output = v["output_tokens"].as_u64().unwrap_or(0) as usize;
                            on_event(SseEvent::Usage { input, output });
                            continue;
                        }
                    }
                }
            }
        }

        Ok(full)
    }

    /// Fetch session messages (for loading history on TUI startup)
    pub async fn session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            messages: Vec<SessionMessage>,
        }
        let resp: Resp = self.get(&format!("/v1/sessions/{}", session_id)).await?;
        Ok(resp.messages)
    }

    /// Fetch the default session ID from gateway status
    pub async fn default_session_id(&self) -> Result<String, String> {
        #[derive(serde::Deserialize)]
        struct Sessions {
            #[serde(default)]
            sessions: Vec<SessionEntry>,
        }
        #[derive(serde::Deserialize)]
        struct SessionEntry {
            id: String,
        }
        let resp: Sessions = self.get("/v1/sessions?limit=1").await?;
        resp.sessions.first()
            .map(|s| s.id.clone())
            .ok_or_else(|| "No sessions found".to_string())
    }

    /// Fetch Pantheon war rooms
    pub async fn pantheon_rooms(&self) -> Result<Vec<PantheonRoomResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            rooms: Vec<PantheonRoomResponse>,
        }
        let resp: Resp = self.get("/v1/pantheon/rooms").await?;
        Ok(resp.rooms)
    }

    /// Fetch Pantheon missions
    pub async fn pantheon_missions(&self) -> Result<Vec<PantheonMissionResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            missions: Vec<PantheonMissionResponse>,
        }
        let resp: Resp = self.get("/v1/pantheon/missions").await?;
        Ok(resp.missions)
    }

    /// Fetch a workspace memory file by path (e.g. "daily/2026-03-27.md")
    pub async fn memory_file(&self, path: &str) -> Result<String, String> {
        let resp = self.client
            .get(format!("{}/v1/memory/files/{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Not found: {}", resp.status()));
        }
        // Response may be JSON with content field or raw text
        let text = resp.text().await.map_err(|e| format!("Read failed: {e}"))?;
        // Try JSON first (gateway wraps in {"content": "..."})
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(content) = v["content"].as_str() {
                return Ok(content.to_string());
            }
        }
        Ok(text)
    }

    /// Fetch messages from a Pantheon room
    pub async fn pantheon_room_messages(&self, room_id: &str) -> Result<Vec<PantheonMessageResponse>, String> {
        #[derive(serde::Deserialize)]
        struct Resp {
            #[serde(default)]
            messages: Vec<PantheonMessageResponse>,
        }
        let resp: Resp = self.get(&format!("/v1/pantheon/rooms/{}/messages?limit=50", room_id)).await?;
        Ok(resp.messages)
    }

    /// Send a message to a Pantheon room
    pub async fn pantheon_send_message(&self, room_id: &str, content: &str, sender: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Req { content: String, sender_id: String, message_type: String }
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/rooms/{}/messages", room_id),
            &Req { content: content.to_string(), sender_id: sender.to_string(), message_type: "chat".to_string() },
        ).await?;
        Ok(())
    }

    /// Create a Pantheon room
    pub async fn pantheon_create_room(&self, name: &str) -> Result<String, String> {
        #[derive(serde::Serialize)]
        struct Req { name: String, created_by: String }
        let resp: serde_json::Value = self.post("/v1/pantheon/rooms", &Req {
            name: name.to_string(),
            created_by: "tui-user".to_string(),
        }).await?;
        Ok(resp["id"].as_str().unwrap_or("").to_string())
    }

    /// Intervene in a Pantheon mission (pause/cancel/redirect)
    pub async fn pantheon_intervene(&self, mission_id: &str, action: &str, reason: &str) -> Result<(), String> {
        #[derive(serde::Serialize)]
        struct Req { action: String, reason: String }
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/missions/{}/intervene", mission_id),
            &Req { action: action.to_string(), reason: reason.to_string() },
        ).await?;
        Ok(())
    }

    /// Approve a Pantheon mission plan
    pub async fn pantheon_approve(&self, mission_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/missions/{}/approve", mission_id),
            &serde_json::json!({}),
        ).await?;
        Ok(())
    }

    /// Approve a plan card (plan-level approval, not mission-level)
    pub async fn pantheon_approve_plan(&self, plan_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/plans/{}/approve", plan_id),
            &serde_json::json!({"approver_id": "tui", "approver_name": "TUI"}),
        ).await?;
        Ok(())
    }

    /// Reject a plan card with an optional reason
    pub async fn pantheon_reject_plan(&self, plan_id: &str, reason: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/pantheon/plans/{}/reject", plan_id),
            &serde_json::json!({"reason": reason, "approver_id": "tui", "approver_name": "TUI"}),
        ).await?;
        Ok(())
    }

    /// Clear a session (remove all messages, keep file)
    pub async fn session_clear(&self, session_id: &str) -> Result<(), String> {
        let _: serde_json::Value = self.post(
            &format!("/v1/sessions/{}/clear", session_id),
            &serde_json::json!({}),
        ).await?;
        Ok(())
    }

    /// Compact a session (strip tool outputs from older messages)
    pub async fn session_compact(&self, session_id: &str) -> Result<String, String> {
        let resp: serde_json::Value = self.post(
            &format!("/v1/sessions/{}/compact", session_id),
            &serde_json::json!({}),
        ).await?;
        Ok(resp["message"].as_str().unwrap_or("Compacted").to_string())
    }

    /// Fetch the full config from the gateway (sanitized — no secrets).
    pub async fn config(&self) -> Result<serde_json::Value, String> {
        self.get("/v1/config").await
    }

    /// Update config fields via PUT /v1/config.
    pub async fn update_config(&self, updates: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.client
            .put(format!("{}/v1/config", self.base_url))
            .json(updates)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }

    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.client
            .get(format!("{}{}", self.base_url, path))
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<T>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }

    async fn post<T: serde::de::DeserializeOwned, B: Serialize>(&self, path: &str, body: &B) -> Result<T, String> {
        self.client
            .post(format!("{}{}", self.base_url, path))
            .json(body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?
            .json::<T>()
            .await
            .map_err(|e| format!("Parse failed: {}", e))
    }
}
