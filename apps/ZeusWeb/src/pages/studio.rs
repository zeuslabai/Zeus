// ═══════════════════════════════════════════════════════════
// ZEUS — Agent Studio (Chat) Page — Full Integration
// WebSocket streaming, Whisper STT, Piper TTS, Reactive Orb
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use std::rc::Rc;
use std::cell::RefCell;
use crate::api;
use crate::components::design::*;
use crate::components::sentient_orb::SentientOrb;
use crate::components::markdown::render_markdown;

// ─── DATA TYPES ─────────────────────────────────────────

#[derive(Clone, Debug)]
struct ChatMessage {
    role: String,       // "user", "assistant", "system"
    text: String,
    tools: Vec<ToolCallInfo>,
    time: String,
    streaming: bool,    // true while still receiving chunks
}

#[derive(Clone, Debug)]
struct ToolCallInfo {
    name: String,
    status: String,     // "running", "success", "failed"
}

#[derive(Clone, Debug)]
struct PuppetAction {
    action_type: String,
    target: String,
    value: String,
    #[allow(dead_code)]
    timestamp: String,
}

// ─── MAIN COMPONENT ────────────────────────────────────

#[component]
pub fn StudioPage() -> impl IntoView {
    // ── State signals ──
    let input = RwSignal::new(String::new());
    let orb_mode = RwSignal::new("dormant".to_string());
    let is_streaming = RwSignal::new(false);
    let is_recording = RwSignal::new(false);
    let tts_enabled = RwSignal::new(true);
    let ws_connected = RwSignal::new(false);
    let is_executing_tool = RwSignal::new(false);
    let messages = RwSignal::new(Vec::<ChatMessage>::new());
    let session_id = RwSignal::new(Option::<String>::None);
    let session_tools = RwSignal::new(Vec::<api::ToolExecution>::new());
    let status = RwSignal::new(api::StatusResponse::default());
    let scroll_trigger = RwSignal::new(0u32);
    let sessions = RwSignal::new(Vec::<api::Session>::new());
    let sidebar_open = RwSignal::new(true);
    let sessions_loading = RwSignal::new(true);
    // ── Puppet panel signals ──
    let puppet_mode = RwSignal::new(false);
    let puppet_actions = RwSignal::new(Vec::<PuppetAction>::new());
    let puppet_screenshot_b64: RwSignal<Option<String>> = RwSignal::new(None);
    let puppet_active = RwSignal::new(false);
    // ── Drive mode + file upload ──
    let drive_mode = RwSignal::new(false);
    let file_input_ref = NodeRef::<leptos::html::Input>::new();

    // WebSocket + MediaRecorder stored in Rc<RefCell> (non-Send, WASM only)
    let ws_ref: Rc<RefCell<Option<web_sys::WebSocket>>> = Rc::new(RefCell::new(None));
    let recorder_ref: Rc<RefCell<Option<web_sys::MediaRecorder>>> = Rc::new(RefCell::new(None));
    let stream_ref: Rc<RefCell<Option<web_sys::MediaStream>>> = Rc::new(RefCell::new(None));

    // Scroll container ref
    let msg_container = NodeRef::<leptos::html::Div>::new();

    // ── Fetch status + sessions on mount ──
    {
        let status = status;
        spawn_local(async move {
            if let Ok(s) = api::fetch_status().await { status.set(s); }
            if let Ok(ss) = api::fetch_sessions().await { sessions.set(ss.sessions); }
            sessions_loading.set(false);
        });
    }

    // ── Auto-scroll when messages change ──
    Effect::new(move |_| {
        let _ = scroll_trigger.get();
        let _ = messages.get();
        let container = msg_container;
        if let Some(el) = container.get() {
            let el: web_sys::Element = el.into();
            el.set_scroll_top(el.scroll_height());
        }
    });

    // ── Connect WebSocket ──
    {
        let ws_ref = ws_ref.clone();
        let messages = messages;
        let orb_mode = orb_mode;
        let is_streaming = is_streaming;
        let ws_connected = ws_connected;
        let tts_enabled = tts_enabled;
        let scroll_trigger = scroll_trigger;

        spawn_local(async move {
            // Small delay to let the page render first
            gloo_timers::future::TimeoutFuture::new(500).await;

            let window = web_sys::window().unwrap();
            let location = window.location();
            let protocol = if location.protocol().unwrap_or_default() == "https:" {
                "wss:"
            } else {
                "ws:"
            };
            let host = location.host().unwrap_or_default();
            // Browser WebSocket can't set Authorization headers;
            // server accepts ?token= query param for auth.
            let url = match api::get_auth_token() {
                Some(tok) => format!("{}//{}/v1/ws?token={}", protocol, host, js_sys::encode_uri_component(&tok)),
                None => format!("{}//{}/v1/ws", protocol, host),
            };

            let Ok(ws) = web_sys::WebSocket::new(&url) else {
                web_sys::console::error_1(&"[Zeus] Failed to create WebSocket".into());
                return;
            };

            // onopen
            let connected = ws_connected;
            let on_open = Closure::wrap(Box::new(move |_: web_sys::Event| {
                connected.set(true);
                web_sys::console::log_1(&"[Zeus] WebSocket connected".into());
            }) as Box<dyn FnMut(_)>);
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            on_open.forget();

            // onclose — auto-reconnect with guard against multiple concurrent attempts
            let connected2 = ws_connected;
            let url_clone = url.clone();
            let ws_ref_close = ws_ref.clone();
            let reconnecting = std::rc::Rc::new(std::cell::Cell::new(false));
            let reconnecting_c = reconnecting.clone();
            let on_close = Closure::wrap(Box::new(move |_: web_sys::CloseEvent| {
                connected2.set(false);
                if reconnecting_c.get() {
                    web_sys::console::log_1(&"[Zeus] WebSocket closed — reconnect already in progress".into());
                    return;
                }
                reconnecting_c.set(true);
                web_sys::console::log_1(&"[Zeus] WebSocket closed — will reconnect in 3s".into());
                let url_r = url_clone.clone();
                let ws_ref_r = ws_ref_close.clone();
                let conn_r = connected2;
                let reconn_r = reconnecting_c.clone();
                spawn_local(async move {
                    gloo_timers::future::TimeoutFuture::new(3_000).await;
                    web_sys::console::log_1(&"[Zeus] Attempting WebSocket reconnect...".into());
                    if let Ok(new_ws) = web_sys::WebSocket::new(&url_r) {
                        let conn_ro = conn_r;
                        let on_open_r = Closure::wrap(Box::new(move |_: web_sys::Event| {
                            conn_ro.set(true);
                            web_sys::console::log_1(&"[Zeus] WebSocket reconnected".into());
                        }) as Box<dyn FnMut(_)>);
                        new_ws.set_onopen(Some(on_open_r.as_ref().unchecked_ref()));
                        on_open_r.forget();
                        *ws_ref_r.borrow_mut() = Some(new_ws);
                    }
                    reconn_r.set(false);
                });
            }) as Box<dyn FnMut(_)>);
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            on_close.forget();

            // onerror
            let on_error = Closure::wrap(Box::new(move |_: web_sys::ErrorEvent| {
                web_sys::console::error_1(&"[Zeus] WebSocket error".into());
            }) as Box<dyn FnMut(_)>);
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            on_error.forget();

            // onmessage — handle all Zeus WS protocol events
            let on_message = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
                let Some(text) = e.data().as_string() else { return };
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&text) else { return };
                let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match msg_type {
                    "started" => {
                        is_streaming.set(true);
                        orb_mode.set("thinking".to_string());
                    }
                    "text_chunk" => {
                        if let Some(chunk) = msg.get("chunk").and_then(|c| c.as_str()) {
                            orb_mode.set("speaking".to_string());
                            messages.update(|m| {
                                if let Some(last) = m.last_mut()
                                    && last.role == "assistant" && last.streaming {
                                        last.text.push_str(chunk);
                                        scroll_trigger.update(|n| *n += 1);
                                        return;
                                    }
                                m.push(ChatMessage {
                                    role: "assistant".to_string(),
                                    text: chunk.to_string(),
                                    tools: vec![],
                                    time: "now".to_string(),
                                    streaming: true,
                                });
                            });
                            scroll_trigger.update(|n| *n += 1);
                        }
                    }
                    "tool_call" => {
                        is_executing_tool.set(true);
                        let name = msg.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        orb_mode.set("thinking".to_string());
                        messages.update(|m| {
                            if let Some(last) = m.last_mut()
                                && last.role == "assistant" {
                                    last.tools.push(ToolCallInfo {
                                        name,
                                        status: "running".to_string(),
                                    });
                                }
                        });
                        scroll_trigger.update(|n| *n += 1);
                    }
                    "tool_result" => {
                        let name = msg.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let success = msg.get("success")
                            .and_then(|s| s.as_bool())
                            .unwrap_or(false);
                        orb_mode.set("speaking".to_string());
                        messages.update(|m| {
                            if let Some(last) = m.last_mut()
                                && last.role == "assistant"
                                    && let Some(tool) = last.tools.iter_mut()
                                        .find(|t| t.name == name && t.status == "running")
                                    {
                                        tool.status = if success {
                                            "success".to_string()
                                        } else {
                                            "failed".to_string()
                                        };
                                    }
                        });
                    }
                    "response_complete" => {
                        if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                            messages.update(|m| {
                                if let Some(last) = m.last_mut()
                                    && last.role == "assistant" {
                                        last.text = content.to_string();
                                        last.streaming = false;
                                    }
                            });
                        }
                        // Capture session_id from WS so POST fallback continues same session
                        if let Some(sid) = msg.get("session_id").and_then(|s| s.as_str()) {
                            if !sid.is_empty() {
                                session_id.set(Some(sid.to_string()));
                            }
                        }
                    }
                    "finished" => {
                        is_streaming.set(false);
                        is_executing_tool.set(false);
                        messages.update(|m| {
                            if let Some(last) = m.last_mut()
                                && last.role == "assistant" {
                                    last.streaming = false;
                                    last.time = "just now".to_string();
                                }
                        });

                        // Trigger TTS if enabled
                        if tts_enabled.get_untracked() {
                            let last_text = messages.get_untracked().last()
                                .filter(|m| m.role == "assistant")
                                .map(|m| m.text.clone());
                            if let Some(ref text) = last_text
                                && !text.is_empty() && !text.starts_with("Error:") {
                                    let t = text.clone();
                                    let orb = orb_mode;
                                    spawn_local(async move {
                                        speak_tts(&t, orb).await;
                                    });
                                    return;
                                }
                        }

                        // No TTS — fade orb to dormant after 2s
                        orb_mode.set("active".to_string());
                        let orb = orb_mode;
                        spawn_local(async move {
                            gloo_timers::future::TimeoutFuture::new(2000).await;
                            if orb.get_untracked() == "active" {
                                orb.set("dormant".to_string());
                            }
                        });
                    }
                    "error" => {
                        is_executing_tool.set(false);
                        let err_msg = msg.get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("Unknown error");
                        messages.update(|m| {
                            // Close any streaming message
                            if let Some(last) = m.last_mut()
                                && last.streaming {
                                    last.streaming = false;
                                }
                            m.push(ChatMessage {
                                role: "system".to_string(),
                                text: format!("Error: {}", err_msg),
                                tools: vec![],
                                time: "now".to_string(),
                                streaming: false,
                            });
                        });
                        is_streaming.set(false);
                        orb_mode.set("dormant".to_string());
                    }
                    "puppet_action" => {
                        let action_type = msg.get("action_type").and_then(|v| v.as_str()).unwrap_or("action").to_string();
                        let target = msg.get("target").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let value = msg.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let timestamp = msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or("now").to_string();
                        puppet_actions.update(|a| a.push(PuppetAction { action_type, target, value, timestamp }));
                        puppet_active.set(true);
                        if let Some(b64) = msg.get("screenshot").and_then(|v| v.as_str()) {
                            puppet_screenshot_b64.set(Some(b64.to_string()));
                        }
                    }
                    "puppet_screenshot" => {
                        if let Some(b64) = msg.get("data").and_then(|v| v.as_str()) {
                            puppet_screenshot_b64.set(Some(b64.to_string()));
                        }
                        puppet_active.set(true);
                    }
                    "puppet_end" => {
                        puppet_active.set(false);
                    }
                    "pong" => {} // keepalive, ignore
                    _ => {
                        web_sys::console::log_1(
                            &format!("[Zeus] Unknown WS message type: {}", msg_type).into()
                        );
                    }
                }
            }) as Box<dyn FnMut(_)>);
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            on_message.forget();

            // Store the WebSocket
            *ws_ref.borrow_mut() = Some(ws);
        });
    }

    // ── Send message (WS or fallback POST) ──
    let ws_for_send = ws_ref.clone();
    let send_message = move || {
        let msg = input.get().trim().to_string();
        if msg.is_empty() || is_streaming.get() {
            return;
        }

        // Slash commands: /clear and /compact
        if msg == "/clear" {
            input.set(String::new());
            messages.set(vec![ChatMessage {
                role: "system".to_string(),
                text: "Session cleared.".to_string(),
                tools: vec![],
                time: "now".to_string(),
                streaming: false,
            }]);
            spawn_local(async move {
                let _: Result<serde_json::Value, String> = api::post_json("/v1/sessions/agent:main:main/clear", &serde_json::json!({})).await;
            });
            return;
        }
        if msg == "/compact" {
            input.set(String::new());
            messages.update(|m| m.push(ChatMessage {
                role: "system".to_string(),
                text: "Compacting session...".to_string(),
                tools: vec![],
                time: "now".to_string(),
                streaming: false,
            }));
            spawn_local(async move {
                let _: Result<serde_json::Value, String> = api::post_json("/v1/sessions/agent:main:main/compact", &serde_json::json!({})).await;
            });
            return;
        }

        // Add user message
        messages.update(|m| {
            m.push(ChatMessage {
                role: "user".to_string(),
                text: msg.clone(),
                tools: vec![],
                time: "now".to_string(),
                streaming: false,
            });
        });
        input.set(String::new());
        scroll_trigger.update(|n| *n += 1);

        // ── Drive Mode: create studio session + autonomous loop ──
        if drive_mode.get() {
            let goal = msg.clone();
            is_streaming.set(true);
            orb_mode.set("thinking".to_string());
            spawn_local(async move {
                match api::create_studio_session(&goal).await {
                    Ok(resp) => {
                        let studio_id = resp.get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if studio_id.is_empty() {
                            messages.update(|m| {
                                m.push(ChatMessage {
                                    role: "system".to_string(),
                                    text: "Error: Failed to create studio session".to_string(),
                                    tools: vec![],
                                    time: "now".to_string(),
                                    streaming: false,
                                });
                            });
                            is_streaming.set(false);
                            orb_mode.set("dormant".to_string());
                            return;
                        }
                        messages.update(|m| {
                            m.push(ChatMessage {
                                role: "system".to_string(),
                                text: format!("\u{1f3d7}\u{fe0f} Studio session created: {}\nStarting autonomous drive...", studio_id),
                                tools: vec![],
                                time: "now".to_string(),
                                streaming: false,
                            });
                        });
                        scroll_trigger.update(|n| *n += 1);
                        // Start driving
                        match api::drive_studio_session(&studio_id, true).await {
                            Ok(drive_resp) => {
                                let drive_status = drive_resp.get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                messages.update(|m| {
                                    m.push(ChatMessage {
                                        role: "assistant".to_string(),
                                        text: format!("Drive status: **{}**\nSession: {}\nThe agent is now working autonomously on your goal.", drive_status, studio_id),
                                        tools: vec![],
                                        time: "just now".to_string(),
                                        streaming: false,
                                    });
                                });
                            }
                            Err(e) => {
                                messages.update(|m| {
                                    m.push(ChatMessage {
                                        role: "system".to_string(),
                                        text: format!("Drive error: {}", e),
                                        tools: vec![],
                                        time: "now".to_string(),
                                        streaming: false,
                                    });
                                });
                            }
                        }
                    }
                    Err(e) => {
                        messages.update(|m| {
                            m.push(ChatMessage {
                                role: "system".to_string(),
                                text: format!("Studio error: {}", e),
                                tools: vec![],
                                time: "now".to_string(),
                                streaming: false,
                            });
                        });
                    }
                }
                is_streaming.set(false);
                orb_mode.set("dormant".to_string());
                scroll_trigger.update(|n| *n += 1);
            });
            return;
        }

        // Try WebSocket first
        let ws_borrow = ws_for_send.borrow();
        let ws_ready = ws_borrow.as_ref()
            .map(|ws| ws.ready_state() == web_sys::WebSocket::OPEN)
            .unwrap_or(false);

        if ws_ready {
            let ws = ws_borrow.as_ref().unwrap();
            let payload = serde_json::json!({
                "type": "chat",
                "message": msg,
                "session_id": session_id.get_untracked()
            });
            let _ = ws.send_with_str(&payload.to_string());
            is_streaming.set(true);
            orb_mode.set("thinking".to_string());
        } else {
            // Fallback to POST /v1/chat
            drop(ws_borrow);
            is_streaming.set(true);
            orb_mode.set("thinking".to_string());
            let sid = session_id.get();
            spawn_local(async move {
                let req = api::DispatchMissionReq {
                    message: msg,
                    session_id: sid,
                    system_prompt: None,
                };
                match api::dispatch_mission(&req).await {
                    Ok(resp) => {
                        if !resp.session_id.is_empty() {
                            session_id.set(Some(resp.session_id));
                        }
                        messages.update(|m| {
                            m.push(ChatMessage {
                                role: "assistant".to_string(),
                                text: resp.response,
                                tools: vec![],
                                time: "just now".to_string(),
                                streaming: false,
                            });
                        });
                        // TTS
                        if tts_enabled.get_untracked() {
                            let last_text = messages.get_untracked().last()
                                .filter(|m| m.role == "assistant")
                                .map(|m| m.text.clone());
                            if let Some(text) = last_text
                                && !text.is_empty() && !text.starts_with("Error:") {
                                    speak_tts(&text, orb_mode).await;
                                    is_streaming.set(false);
                                    return;
                                }
                        }
                    }
                    Err(e) => {
                        messages.update(|m| {
                            m.push(ChatMessage {
                                role: "system".to_string(),
                                text: format!("Error: {}", e),
                                tools: vec![],
                                time: "now".to_string(),
                                streaming: false,
                            });
                        });
                    }
                }
                is_streaming.set(false);
                orb_mode.set("dormant".to_string());
            });
        }
    };

    // ── Voice recording toggle ──
    let rec_ref = recorder_ref.clone();
    let str_ref = stream_ref.clone();
    let toggle_mic = move |_| {
        if is_recording.get() {
            // Stop recording
            if let Some(recorder) = rec_ref.borrow().as_ref() {
                let _ = recorder.stop();
            }
            if let Some(stream) = str_ref.borrow().as_ref() {
                for track in stream.get_audio_tracks().iter() {
                    if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
                        track.stop();
                    }
                }
            }
            *rec_ref.borrow_mut() = None;
            *str_ref.borrow_mut() = None;
            // is_recording will be set to false by the onstop callback
        } else {
            // Start recording
            let rec_ref2 = recorder_ref.clone();
            let str_ref2 = stream_ref.clone();
            let input_sig = input;
            let is_rec = is_recording;
            let orb = orb_mode;
            spawn_local(async move {
                let window = web_sys::window().unwrap();
                let navigator = window.navigator();
                let Ok(media_devices) = navigator.media_devices() else {
                    web_sys::console::error_1(&"No media devices available".into());
                    return;
                };

                let constraints = web_sys::MediaStreamConstraints::new();
                constraints.set_audio(&JsValue::TRUE);

                let Ok(promise) = media_devices.get_user_media_with_constraints(&constraints) else {
                    web_sys::console::error_1(&"getUserMedia failed".into());
                    return;
                };

                let Ok(stream_js) = wasm_bindgen_futures::JsFuture::from(promise).await else {
                    web_sys::console::error_1(&"Media stream promise rejected".into());
                    return;
                };

                let Ok(stream) = stream_js.dyn_into::<web_sys::MediaStream>() else { return };

                let options = web_sys::MediaRecorderOptions::new();
                options.set_mime_type("audio/webm");

                let Ok(recorder) = web_sys::MediaRecorder::new_with_media_stream_and_media_recorder_options(
                    &stream, &options,
                ) else {
                    web_sys::console::error_1(&"MediaRecorder creation failed".into());
                    return;
                };

                // Collect audio chunks
                let chunks: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(vec![]));
                let chunks_c = chunks.clone();
                let on_data = Closure::wrap(Box::new(move |e: web_sys::BlobEvent| {
                    if let Some(data) = e.data() {
                        chunks_c.borrow_mut().push(data.into());
                    }
                }) as Box<dyn FnMut(_)>);
                recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
                on_data.forget();

                // On stop: combine chunks → send to Whisper → fill input
                let on_stop = Closure::wrap(Box::new(move || {
                    let borrowed = chunks.borrow();
                    let blob_parts = js_sys::Array::new();
                    for chunk in borrowed.iter() {
                        blob_parts.push(chunk);
                    }
                    let mut opts = web_sys::BlobPropertyBag::new();
                    opts.type_("audio/webm");
                    let Ok(blob) = web_sys::Blob::new_with_buffer_source_sequence_and_options(
                        &blob_parts, &opts,
                    ) else { return };

                    let input_s = input_sig;
                    let is_r = is_rec;
                    let orb_s = orb;
                    spawn_local(async move {
                        orb_s.set("thinking".to_string());
                        match transcribe_audio(&blob).await {
                            Ok(text) => {
                                if !text.is_empty() {
                                    input_s.set(text);
                                }
                            }
                            Err(e) => {
                                web_sys::console::error_1(
                                    &format!("[Zeus] Whisper error: {}", e).into()
                                );
                            }
                        }
                        is_r.set(false);
                        orb_s.set("dormant".to_string());
                    });
                }) as Box<dyn FnMut()>);
                recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
                on_stop.forget();

                let _ = recorder.start();
                *rec_ref2.borrow_mut() = Some(recorder);
                *str_ref2.borrow_mut() = Some(stream);
                is_rec.set(true);
                orb.set("listening".to_string());
            });
        }
    };

    // ── TTS toggle handler ──
    let toggle_tts = move |_| {
        tts_enabled.update(|v| *v = !*v);
    };

    // ── View ──
    // ── Load session handler ──
    let load_session = move |sid: String| {
        let sid2 = sid.clone();
        let sid3 = sid.clone();
        session_id.set(Some(sid.clone()));
        messages.set(Vec::new());
        session_tools.set(Vec::new());
        orb_mode.set("thinking".to_string());
        // Fetch tool execution chain for this session
        spawn_local(async move {
            if let Ok(t) = api::fetch_session_tools(&sid3).await { session_tools.set(t.tools); }
        });
        spawn_local(async move {
            if let Ok(detail) = api::fetch_session(&sid2).await {
                let msgs: Vec<ChatMessage> = detail.messages.into_iter().map(|m| {
                    ChatMessage {
                        role: m.role.clone(),
                        text: m.content.clone(),
                        tools: m.tool_calls.iter().filter_map(|tc| {
                            tc.get("name").and_then(|n| n.as_str()).map(|n| ToolCallInfo {
                                name: n.to_string(),
                                status: "success".to_string(),
                            })
                        }).collect(),
                        time: m.timestamp.clone(),
                        streaming: false,
                    }
                }).collect();
                messages.set(msgs);
            }
            orb_mode.set("dormant".to_string());
        });
    };

    // ── Keyboard shortcuts ──
    {
        spawn_local(async move {
            let window = web_sys::window().unwrap();
            let cb = Closure::wrap(Box::new(move |ev: web_sys::KeyboardEvent| {
                // Ctrl+N or Cmd+N: new session
                if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "n" {
                    ev.prevent_default();
                    messages.set(Vec::new());
                    session_id.set(None);
                    input.set(String::new());
                    orb_mode.set("dormant".to_string());
                }
                // Escape: clear input
                if ev.key() == "Escape" {
                    input.set(String::new());
                }
            }) as Box<dyn FnMut(_)>);
            window.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref()).ok();
            cb.forget();
        });
    }

    view! {
        <div style="display: flex; height: 100vh;">
            // ── Session Sidebar ──
            <Show when=move || sidebar_open.get()>
                <div style="width: 260px; border-right: 1px solid rgba(255,60,20,0.1); display: flex; flex-direction: column; flex-shrink: 0; background: rgba(0,0,0,0.3);">
                    <div style="padding: 16px; border-bottom: 1px solid rgba(255,60,20,0.08);">
                        <button
                            on:click=move |_| {
                                messages.set(Vec::new());
                                session_id.set(None);
                                input.set(String::new());
                                orb_mode.set("dormant".to_string());
                            }
                            style="width: 100%; padding: 10px; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.3); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; display: flex; align-items: center; justify-content: center; gap: 8px;"
                        >
                            <Icon name="plus" size=12 /> "NEW SESSION"
                        </button>
                    </div>
                    <div style="flex: 1; overflow-y: auto; padding: 8px;">
                        {move || {
                            if sessions_loading.get() {
                                return vec![view! {
                                    <div style="text-align: center; padding: 24px; color: rgba(255,245,240,0.5); font-size: 11px;">"Loading sessions..."</div>
                                }.into_any()];
                            }
                            let ss = sessions.get();
                            if ss.is_empty() {
                                return vec![view! {
                                    <div style="text-align: center; padding: 24px; color: rgba(255,245,240,0.2); font-size: 11px;">"No previous sessions"</div>
                                }.into_any()];
                            }
                            ss.into_iter().take(30).map(|s| {
                                let sid = s.id.clone();
                                let is_current = session_id.get() == Some(sid.clone());
                                let load = load_session;
                                view! {
                                    <div
                                        on:click={let sid = sid.clone(); move |_| load(sid.clone())}
                                        style=move || format!(
                                            "padding: 10px 12px; margin-bottom: 4px; border-radius: 8px; cursor: pointer; transition: all 0.15s; border: 1px solid {}; background: {};",
                                            if is_current { "rgba(255,60,20,0.25)" } else { "transparent" },
                                            if is_current { "rgba(255,60,20,0.06)" } else { "transparent" },
                                        )
                                    >
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.85); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">
                                            {if !s.agent_name.is_empty() { s.agent_name.clone() } else { format!("Session {}", &sid[..8.min(sid.len())]) }}
                                        </div>
                                        <div style="display: flex; justify-content: space-between; margin-top: 4px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                            <span>{format!("{} msgs", s.message_count)}</span>
                                            <span>{s.created.clone()}</span>
                                        </div>
                                    </div>
                                }.into_any()
                            }).collect::<Vec<_>>()
                        }}
                    </div>
                </div>
            </Show>

            // ── Main Chat Area ──
            <div style="flex: 1; display: flex; flex-direction: column;">
            // ── Header ──
            <div style="padding: 16px 24px; border-bottom: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; gap: 16px; flex-shrink: 0;">
                // Sidebar toggle
                <button
                    on:click=move |_| sidebar_open.update(|v| *v = !*v)
                    style="width: 36px; height: 36px; border-radius: 8px; border: 1px solid rgba(255,60,20,0.1); background: rgba(255,255,255,0.03); color: rgba(255,245,240,0.4); cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0;"
                    title="Toggle session sidebar"
                >
                    <Icon name="sessions" size=16 />
                </button>
                {move || view! { <SentientOrb size=40 mode={orb_mode.get()} /> }}
                <div style="flex: 1;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                        "AGENT STUDIO"
                    </div>
                    <div style="font-size: 11px; color: rgba(255,245,240,0.7);">
                        {move || {
                            let s = status.get();
                            let model = if s.model.is_empty() { "connecting...".to_string() } else { s.model.clone() };
                            let sid = session_id.get().unwrap_or_else(|| "new".to_string());
                            let ws_icon = if ws_connected.get() { "⚡" } else { "⏳" };
                            format!("{} {} • Session {}", ws_icon, model, sid)
                        }}
                    </div>
                </div>
                // TTS toggle
                <button
                    on:click=toggle_tts
                    title="Toggle voice output (Piper TTS)"
                    style=move || format!(
                        "width: 36px; height: 36px; border-radius: 8px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; display: flex; align-items: center; justify-content: center; transition: all 0.2s; font-size: 16px;",
                        if tts_enabled.get() { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" },
                        if tts_enabled.get() { "rgba(255,60,20,0.20)" } else { "rgba(255,255,255,0.03)" },
                        if tts_enabled.get() { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
                    )
                >
                    <Icon name="voice" size=16 />
                </button>
                // Puppet view toggle
                <button
                    on:click=move |_| {
                        let next = !puppet_mode.get_untracked();
                        puppet_mode.set(next);
                        if next { puppet_actions.set(Vec::new()); }
                    }
                    title="Toggle puppet view — watch Zeus drive the browser"
                    style=move || format!(
                        "width: 36px; height: 36px; border-radius: 8px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; display: flex; align-items: center; justify-content: center; transition: all 0.2s; font-size: 16px;",
                        if puppet_mode.get() { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" },
                        if puppet_mode.get() { "rgba(255,60,20,0.20)" } else { "rgba(255,255,255,0.03)" },
                        if puppet_mode.get() { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
                    )
                >"\u{1f3ae}"</button>
                // Drive mode toggle
                <button
                    on:click=move |_| drive_mode.update(|v| *v = !*v)
                    title="Toggle Drive Mode (autonomous task execution)"
                    style=move || format!(
                        "padding: 6px 12px; border-radius: 8px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; transition: all 0.2s;",
                        if drive_mode.get() { "rgba(34,197,94,0.4)" } else { "rgba(255,60,20,0.1)" },
                        if drive_mode.get() { "rgba(34,197,94,0.15)" } else { "rgba(255,255,255,0.03)" },
                        if drive_mode.get() { "rgba(34,197,94,1)" } else { "rgba(255,245,240,0.5)" },
                    )
                >
                    {move || if drive_mode.get() { "\u{26a1} DRIVE MODE" } else { "\u{1f4ac} CHAT MODE" }}
                </button>
                // Status badge
                {move || {
                    let (text, color) = if is_recording.get() {
                        ("Recording", "rgba(239,68,68,1)")
                    } else if is_streaming.get() {
                        ("Streaming", "rgba(234,179,8,1)")
                    } else if ws_connected.get() {
                        ("Connected", "rgba(34,197,94,1)")
                    } else {
                        ("Offline", "rgba(255,245,240,0.5)")
                    };
                    view! { <Badge text={text} color={color} /> }
                }}
            </div>

            // ── "Zeus is driving" banner ──
            <Show when=move || is_executing_tool.get()>
                <div style="padding: 7px 24px; background: rgba(255,60,20,0.06); border-bottom: 1px solid rgba(255,60,20,0.15); display: flex; align-items: center; gap: 10px; flex-shrink: 0;">
                    <div style="width: 7px; height: 7px; border-radius: 50%; background: rgba(255,60,20,0.85); animation: pulse 0.9s ease-in-out infinite; flex-shrink: 0;" />
                    <span style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,140,80,0.9);">"\u{1f3ae} ZEUS IS DRIVING"</span>
                    {move || {
                        let tool_name = messages.get().last()
                            .and_then(|m| m.tools.iter().find(|t| t.status == "running"))
                            .map(|t| t.name.clone())
                            .unwrap_or_default();
                        if !tool_name.is_empty() {
                            view! { <span style="font-size: 11px; color: rgba(255,245,240,0.3);">"executing: "{tool_name}</span> }.into_any()
                        } else {
                            view! { <span /> }.into_any()
                        }
                    }}
                </div>
            </Show>

            // ── Messages ──
            <div
                node_ref=msg_container
                style="flex: 1; overflow-y: auto; padding: 24px; display: flex; flex-direction: column; gap: 16px;"
            >
                {move || {
                    let msgs = messages.get();
                    if msgs.is_empty() {
                        vec![view! {
                            <div style="flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; opacity: 0.5;">
                                <SentientOrb size=100 mode="dormant" />
                                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5);">
                                    "AWAITING INSTRUCTIONS"
                                </div>
                                <div style="font-size: 12px; color: rgba(255,245,240,0.7); text-align: center; max-width: 400px; line-height: 1.6;">
                                    {move || if ws_connected.get() {
                                        "WebSocket connected — responses stream in real-time with tool call visibility"
                                    } else {
                                        "Connecting to Zeus gateway..."
                                    }}
                                </div>
                            </div>
                        }.into_any()]
                    } else {
                        msgs.into_iter().map(|m| {
                            let is_user = m.role == "user";
                            let is_system = m.role == "system";
                            let tools = m.tools.clone();
                            let text = m.text.clone();
                            let time = m.time.clone();
                            let still_streaming = m.streaming;

                            view! {
                                <div style={
                                    if is_system {
                                        "display: flex; justify-content: center;"
                                    } else if is_user {
                                        "display: flex; gap: 12px; flex-direction: row-reverse;"
                                    } else {
                                        "display: flex; gap: 12px; flex-direction: row;"
                                    }
                                }>
                                    // Assistant orb avatar
                                    {(!is_user && !is_system).then(|| view! {
                                        <div style="flex-shrink: 0; margin-top: 4px;">
                                            <SentientOrb size=32 mode={if still_streaming { "speaking" } else { "dormant" }} />
                                        </div>
                                    })}
                                    // Message bubble
                                    <div style={
                                        if is_system {
                                            "max-width: 80%; padding: 8px 16px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 8px;"
                                        } else if is_user {
                                            "max-width: 70%; padding: 12px 16px; background: rgba(255,60,20,0.08); border: 1px solid rgba(255,60,20,0.2); border-radius: 16px 16px 4px 16px;"
                                        } else {
                                            "max-width: 85%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 16px 16px 16px 4px;"
                                        }
                                    }>
                                        // Message text (markdown for assistant, plain for user/system)
                                        {if !is_user && !is_system {
                                            view! {
                                                <div class="zeus-md" inner_html={render_markdown(&text)} />
                                                {still_streaming.then(|| view! {
                                                    <span style="display: inline-block; width: 8px; height: 14px; background: rgba(255,60,20,0.6); margin-left: 2px; animation: pulse 1s ease-in-out infinite;" />
                                                })}
                                            }.into_any()
                                        } else {
                                            view! {
                                                <div style="font-size: 14px; color: rgba(255,245,240,0.9); line-height: 1.6; white-space: pre-wrap; word-break: break-word;">
                                                    {text}
                                                </div>
                                            }.into_any()
                                        }}
                                        // Tool calls
                                        {(!tools.is_empty()).then(|| view! {
                                            <div style="display: flex; gap: 4px; margin-top: 8px; flex-wrap: wrap;">
                                                {tools.iter().map(|t| {
                                                    let color = match t.status.as_str() {
                                                        "running" => "rgba(234,179,8,0.7)",
                                                        "success" => "rgba(34,197,94,0.7)",
                                                        "failed" => "rgba(239,68,68,0.7)",
                                                        _ => "rgba(255,140,80,0.5)",
                                                    };
                                                    let icon = match t.status.as_str() {
                                                        "running" => "...",
                                                        "success" => "✓",
                                                        "failed" => "✗",
                                                        _ => "",
                                                    };
                                                    let label = format!("{} {}", t.name, icon);
                                                    view! {
                                                        <Badge text={label} color={color.to_string()} />
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        })}
                                        // Timestamp
                                        <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 6px;">
                                            {time}
                                        </div>
                                    </div>
                                </div>
                            }.into_any()
                        }).collect::<Vec<_>>()
                    }
                }}

                // Streaming indicator (typing dots)
                <Show when=move || is_streaming.get() && messages.get().last().map(|m| !m.streaming).unwrap_or(true)>
                    <div style="display: flex; gap: 12px;">
                        <div style="flex-shrink: 0; margin-top: 4px;">
                            <SentientOrb size=32 mode="thinking" />
                        </div>
                        <div style="max-width: 85%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 16px 16px 16px 4px;">
                            <div style="display: flex; gap: 4px;">
                                <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.6); animation: pulse 1.5s ease-in-out 0s infinite;" />
                                <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.6); animation: pulse 1.5s ease-in-out 0.2s infinite;" />
                                <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.6); animation: pulse 1.5s ease-in-out 0.4s infinite;" />
                            </div>
                        </div>
                    </div>
                </Show>
            </div>

            // ── Input Area ──
            <div style="padding: 16px 24px; border-top: 1px solid rgba(255,60,20,0.1); flex-shrink: 0;">
                <div style="display: flex; gap: 10px; align-items: flex-end;">
                    // Mic button (Whisper STT)
                    <button
                        on:click=toggle_mic
                        title="Voice input (Whisper STT)"
                        style=move || format!(
                            "width: 44px; height: 44px; border-radius: 10px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0; transition: all 0.2s;",
                            if is_recording.get() { "rgba(239,68,68,0.5)" } else { "rgba(255,60,20,0.1)" },
                            if is_recording.get() { "rgba(239,68,68,0.15)" } else { "rgba(255,255,255,0.03)" },
                            if is_recording.get() { "rgba(239,68,68,1)" } else { "rgba(255,245,240,0.5)" },
                        )
                    >
                        <Icon name="voice" size=18 />
                    </button>
                    // Upload button (file attachment)
                    <button
                        on:click=move |_| {
                            if let Some(el) = file_input_ref.get() {
                                let el: web_sys::HtmlElement = el.into();
                                el.click();
                            }
                        }
                        title="Attach file"
                        style="width: 44px; height: 44px; border-radius: 10px; border: 1px solid rgba(255,60,20,0.1); background: rgba(255,255,255,0.03); color: rgba(255,245,240,0.5); cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0; transition: all 0.2s;"
                    >
                        // Paperclip SVG inline
                        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
                        </svg>
                    </button>
                    // Hidden file input
                    <input
                        node_ref=file_input_ref
                        type="file"
                        style="display: none;"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let input_el: web_sys::HtmlInputElement = target.dyn_into().unwrap();
                            if let Some(files) = input_el.files() {
                                if let Some(file) = files.get(0) {
                                    let fname = file.name();
                                    messages.update(|m| {
                                        m.push(ChatMessage {
                                            role: "system".to_string(),
                                            text: format!("\u{1f4ce} Uploading: {}...", fname),
                                            tools: vec![],
                                            time: "now".to_string(),
                                            streaming: false,
                                        });
                                    });
                                    scroll_trigger.update(|n| *n += 1);
                                    spawn_local(async move {
                                        match api::upload_file(file, |_| {}).await {
                                            Ok(uploaded) => {
                                                messages.update(|m| {
                                                    m.push(ChatMessage {
                                                        role: "system".to_string(),
                                                        text: format!("\u{2705} Uploaded: {} ({})", uploaded.name, uploaded.id),
                                                        tools: vec![],
                                                        time: "now".to_string(),
                                                        streaming: false,
                                                    });
                                                });
                                            }
                                            Err(e) => {
                                                messages.update(|m| {
                                                    m.push(ChatMessage {
                                                        role: "system".to_string(),
                                                        text: format!("\u{274c} Upload failed: {}", e),
                                                        tools: vec![],
                                                        time: "now".to_string(),
                                                        streaming: false,
                                                    });
                                                });
                                            }
                                        }
                                        scroll_trigger.update(|n| *n += 1);
                                    });
                                }
                            }
                        }
                    />
                    // Text input (auto-resize)
                    <div style="flex: 1;">
                        <textarea
                            prop:value=move || input.get()
                            on:input=move |ev: web_sys::Event| {
                                let val = event_target_value(&ev);
                                input.set(val);
                                // Auto-resize
                                if let Some(target) = ev.target()
                                    && let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                                        el.style().set_property("height", "auto").ok();
                                        let sh = el.scroll_height();
                                        let h = sh.clamp(44, 150);
                                        el.style().set_property("height", &format!("{}px", h)).ok();
                                    }
                            }
                            on:keydown={
                                let send = send_message.clone();
                                move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" && !ev.shift_key() {
                                        ev.prevent_default();
                                        send();
                                        // Reset height after send
                                        if let Some(target) = ev.target()
                                            && let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                                                el.style().set_property("height", "44px").ok();
                                            }
                                    }
                                }
                            }
                            placeholder="Message Zeus... (Enter to send, Shift+Enter for newline)"
                            rows="1"
                            style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; outline: none; resize: none; box-sizing: border-box; min-height: 44px; max-height: 150px; overflow-y: auto; transition: height 0.1s;"
                        />
                    </div>
                    // Send button
                    <button
                        on:click={
                            let send = send_message.clone();
                            move |_| send()
                        }
                        style=move || format!(
                            "width: 44px; height: 44px; border-radius: 10px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0; transition: all 0.2s;",
                            if input.get().trim().is_empty() || is_streaming.get() { "rgba(255,60,20,0.1)" } else { "rgba(255,60,20,0.4)" },
                            if input.get().trim().is_empty() || is_streaming.get() { "rgba(255,255,255,0.03)" } else { "rgba(255,60,20,0.2)" },
                            if input.get().trim().is_empty() || is_streaming.get() { "rgba(255,245,240,0.5)" } else { "#ff3c14" },
                        )
                    >
                        <Icon name="send" size=18 />
                    </button>
                </div>
                // Input bar footer
                <div style="display: flex; justify-content: space-between; align-items: center; margin-top: 6px; padding: 0 4px;">
                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">
                        {move || if is_recording.get() {
                            "🔴 Recording... click mic to stop & transcribe".to_string()
                        } else if ws_connected.get() {
                            "⚡ Streaming • Ctrl+N new session • Esc clear".to_string()
                        } else {
                            "📡 Connecting... (fallback to POST mode)".to_string()
                        }}
                    </div>
                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">
                        {move || if tts_enabled.get() { "🔊 TTS ON" } else { "🔇 TTS OFF" }}
                    </div>
                </div>
            </div>
            </div> // close main chat area

            // ── Tool Chain Panel ── (shows after session load)
            <Show when=move || !session_tools.get().is_empty()>
                <div style="width: 240px; border-left: 1px solid rgba(255,60,20,0.1); display: flex; flex-direction: column; flex-shrink: 0; background: rgba(0,0,0,0.2);">
                    <div style="padding: 12px 14px; border-bottom: 1px solid rgba(255,60,20,0.08); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,60,20,0.7);">"TOOL CHAIN"</div>
                    <div style="flex: 1; overflow-y: auto; padding: 8px;">
                        {move || session_tools.get().into_iter().map(|t| {
                            let ok_color = if t.success { "#22c55e" } else { "#ef4444" };
                            let dur = if t.duration_ms > 0 { format!("{}ms", t.duration_ms) } else { String::new() };
                            view! {
                                <div style="padding: 8px; margin-bottom: 6px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px;">
                                    <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 4px;">
                                        <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {}; flex-shrink: 0;", ok_color)} />
                                        <span style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; color: rgba(255,245,240,0.9); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{t.name.clone()}</span>
                                    </div>
                                    {(!dur.is_empty()).then(|| view! {
                                        <div style="font-size: 10px; color: rgba(255,245,240,0.4);">{dur}</div>
                                    })}
                                    {(!t.output.is_empty()).then(|| {
                                        let preview: String = t.output.chars().take(60).collect();
                                        let preview = if t.output.len() > 60 { format!("{}…", preview) } else { preview };
                                        view! {
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{preview}</div>
                                        }
                                    })}
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
            </Show>

            // ── Puppet Panel ── (Phase 5: watch Zeus drive the browser)
            <Show when=move || puppet_mode.get()>
                <div style="width: 50%; border-left: 1px solid rgba(255,60,20,0.12); display: flex; flex-direction: column; flex-shrink: 0; background: rgba(0,0,0,0.2); min-width: 320px;">
                    // Puppet header
                    <div style="padding: 14px 18px; border-bottom: 1px solid rgba(255,60,20,0.08); display: flex; align-items: center; gap: 10px; flex-shrink: 0;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,60,20,0.7);">"PUPPET VIEW"</div>
                        <Show when=move || puppet_active.get()>
                            <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.85); animation: pulse 0.9s ease-in-out infinite;" />
                            <span style="font-size: 10px; color: rgba(255,140,80,0.6); font-family: 'Orbitron', monospace; letter-spacing: 1px;">"LIVE"</span>
                        </Show>
                        <div style="flex: 1;" />
                        <button
                            on:click=move |_| { puppet_actions.set(Vec::new()); puppet_screenshot_b64.set(None); }
                            style="padding: 4px 10px; background: transparent; border: 1px solid rgba(255,245,240,0.08); border-radius: 5px; color: rgba(255,245,240,0.3); font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; cursor: pointer;"
                        >"CLEAR"</button>
                    </div>
                    // Screenshot frame
                    <div style="flex: 1; overflow: hidden; background: #0a0a0a; display: flex; align-items: center; justify-content: center; min-height: 0;">
                        {move || {
                            if let Some(b64) = puppet_screenshot_b64.get() {
                                view! {
                                    <img
                                        src={format!("data:image/png;base64,{}", b64)}
                                        style="width: 100%; height: 100%; object-fit: contain; display: block;"
                                    />
                                }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; padding: 24px; text-align: center;">
                                        <div style="font-size: 40px; opacity: 0.12;">"🖥"</div>
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.15);">"AWAITING PUPPET FEED"</div>
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.2); max-width: 220px; line-height: 1.5;">"Ask Zeus to open a website or run a browser task — the live view will appear here"</div>
                                    </div>
                                }.into_any()
                            }
                        }}
                    </div>
                    // Action feed
                    <div style="height: 200px; border-top: 1px solid rgba(255,60,20,0.08); display: flex; flex-direction: column; overflow: hidden; flex-shrink: 0;">
                        <div style="padding: 8px 14px 4px; font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 2px; color: rgba(255,245,240,0.5); flex-shrink: 0;">"ACTION LOG"</div>
                        <div style="flex: 1; overflow-y: auto; padding: 0 14px 8px; display: flex; flex-direction: column; gap: 3px;">
                            {move || {
                                let actions = puppet_actions.get();
                                if actions.is_empty() {
                                    return vec![view! {
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.15); padding: 8px 0;">"No actions yet — ask Zeus to drive the browser"</div>
                                    }.into_any()];
                                }
                                actions.into_iter().rev().take(30).map(|a| {
                                    let (icon, color) = match a.action_type.as_str() {
                                        "click"      => ("\u{1f5b1}", "rgba(59,130,246,0.8)"),
                                        "type"       => ("\u{2328}",  "rgba(34,197,94,0.8)"),
                                        "navigate"   => ("\u{1f310}", "rgba(168,85,247,0.8)"),
                                        "scroll"     => ("\u{2195}",  "rgba(234,179,8,0.8)"),
                                        "screenshot" => ("\u{1f4f8}", "rgba(255,140,80,0.8)"),
                                        _            => ("\u{25b6}",  "rgba(255,245,240,0.4)"),
                                    };
                                    let val_preview = if a.value.len() > 24 {
                                        format!("\"{}...\"", &a.value[..24])
                                    } else if !a.value.is_empty() {
                                        format!("\"{}\"", a.value)
                                    } else {
                                        String::new()
                                    };
                                    let atype = a.action_type.to_uppercase();
                                    let atarget = a.target.clone();
                                    view! {
                                        <div style="display: flex; gap: 6px; align-items: baseline; font-size: 11px; flex-shrink: 0; padding: 1px 0;">
                                            <span>{icon}</span>
                                            <span style={format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; color: {};", color)}>{atype}</span>
                                            {(!atarget.is_empty()).then(|| view! {
                                                <span style="color: rgba(255,245,240,0.4); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 160px;">{atarget}</span>
                                            })}
                                            {(!val_preview.is_empty()).then(|| view! {
                                                <span style="color: rgba(255,245,240,0.5); font-style: italic;">{val_preview}</span>
                                            })}
                                        </div>
                                    }.into_any()
                                }).collect::<Vec<_>>()
                            }}
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

// ─── PIPER TTS ──────────────────────────────────────────
// Sends text to Zeus gateway /v1/tts/synthesize, plays audio

async fn speak_tts(text: &str, orb_mode: RwSignal<String>) {
    orb_mode.set("speaking".to_string());

    // Limit TTS to first 500 chars
    let tts_text = if text.len() > 500 { &text[..{let mut i=500.min(text.len()); while !text.is_char_boundary(i){i-=1;} i}] } else { text };

    let body = serde_json::json!({
        "text": tts_text,
        "voice": "en_US-ryan-medium",
        "format": "mp3"
    });

    let result: Result<(), String> = async {
        let json: serde_json::Value = api::post_json("/v1/tts/synthesize", &body).await?;

        let audio_b64 = json
            .get("audio_base64")
            .and_then(|v| v.as_str())
            .ok_or("No audio_base64 in response")?;

        let data_url = format!("data:audio/mp3;base64,{}", audio_b64);
        let audio = web_sys::HtmlAudioElement::new_with_src(&data_url)
            .map_err(|_| "HtmlAudioElement creation failed")?;

        // When audio ends, set orb to dormant
        let orb = orb_mode;
        let on_ended = Closure::wrap(Box::new(move || {
            orb.set("dormant".to_string());
        }) as Box<dyn FnMut()>);
        audio.set_onended(Some(on_ended.as_ref().unchecked_ref()));
        on_ended.forget();

        let _ = audio.play();
        Ok(())
    }
    .await;

    if result.is_err() {
        orb_mode.set("dormant".to_string());
    }
}

// ─── WHISPER STT ────────────────────────────────────────
// Sends audio blob to Whisper STT endpoint for transcription.
// URL comes from gateway /v1/status or falls back to /stt/ reverse proxy.

async fn transcribe_audio(blob: &web_sys::Blob) -> Result<String, String> {
    let form = web_sys::FormData::new().map_err(|_| "FormData creation failed")?;
    form.append_with_blob_and_filename("file", blob, "recording.webm")
        .map_err(|_| "append blob failed")?;
    form.append_with_str("temperature", "0.0")
        .map_err(|_| "append temp failed")?;
    form.append_with_str("response_format", "json")
        .map_err(|_| "append format failed")?;

    // Use relative /stt/ path (proxied by nginx) — no hardcoded external URL
    let whisper_url = "/stt/inference";

    let window = web_sys::window().unwrap();
    let mut init = web_sys::RequestInit::new();
    init.method("POST");
    init.body(Some(&form));

    let request = web_sys::Request::new_with_str_and_init(whisper_url, &init)
        .map_err(|_| "Request creation failed")?;

    let resp_js = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|_| "Fetch failed — check Whisper STT endpoint and CORS config")?;

    let resp: web_sys::Response = resp_js
        .dyn_into()
        .map_err(|_| "Response cast failed")?;

    if !resp.ok() {
        return Err(format!("Whisper returned status {}", resp.status()));
    }

    let text_promise = resp.text().map_err(|_| "text() failed")?;
    let text_js = wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|_| "text promise failed")?;

    let text = text_js.as_string().ok_or("text not a string")?;

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("JSON parse: {}", e))?;

    json.get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "No 'text' field in Whisper response".to_string())
}
