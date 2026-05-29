//! Inbound Channel Message Processor
//!
//! Background task that reads incoming messages from connected channels
//! (Telegram, Discord, Slack, etc.) via the ChannelManager receiver and
//! routes them to the appropriate agent for processing. Responses are
//! sent back through the originating channel.
//!
//! ## Architecture
//!
//! ```text
//! [Telegram] ─┐
//! [Discord]  ─┤── ChannelManager (mpsc) ──> InboundProcessor
//! [Slack]    ─┘                                   │
//!                                          AgentRegistry.route()
//!                                                 │
//!                                           Agent.run(prompt)
//!                                                 │
//!                                     ChannelManager.send(response)
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use zeus_channels::media_extract::{
    extract_text_from_doc, is_extractable_doc, ExtractError,
};
use zeus_channels::{ChannelAttachment, ChannelManager, ChannelMessage};

use crate::SharedState;

/// Maximum bytes we'll eagerly download from a remote attachment URL.
/// Discord caps free uploads at 25 MiB and Nitro at 100 MiB. We cap at
/// 32 MiB to cover the common case without unbounded memory pressure.
const MAX_ATTACHMENT_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

/// Configuration for the inbound message processor.
#[derive(Debug, Clone)]
pub struct InboundConfig {
    /// Maximum message length to process (bytes). Longer messages are truncated.
    pub max_message_len: usize,
    /// Whether to send typing indicators while processing.
    pub send_typing: bool,
    /// Whether to log inbound messages to Athena.
    pub log_to_athena: bool,
}

impl Default for InboundConfig {
    fn default() -> Self {
        Self {
            max_message_len: zeus_core::MAX_INBOUND_MESSAGE_BYTES,
            send_typing: true,
            log_to_athena: true,
        }
    }
}

/// Start the inbound message processing loop.
///
/// This spawns a tokio task that reads from the `ChannelManager`'s receiver
/// and routes each message to the appropriate agent. Runs until the receiver
/// is closed (all senders dropped) or the server shuts down.
///
/// Returns a `JoinHandle` for the background task.
pub async fn start_inbound_loop(
    state: SharedState,
    channel_manager: Arc<ChannelManager>,
    config: InboundConfig,
) -> tokio::task::JoinHandle<()> {
    // Take the receiver from the channel manager
    let rx = {
        channel_manager.take_receiver()
    };

    let Some(mut rx) = rx else {
        warn!("ChannelManager receiver already taken — inbound loop not started");
        return tokio::spawn(async {});
    };

    info!("Starting inbound channel message processor");

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let state = state.clone();
            let channel_mgr = channel_manager.clone();
            let cfg = config.clone();

            // Process each message in its own task to avoid blocking the loop
            tokio::spawn(async move {
                if let Err(e) = process_message(&state, &channel_mgr, &cfg, msg).await {
                    error!("Failed to process inbound message: {}", e);
                }
            });
        }

        info!("Inbound channel message processor stopped (channel closed)");
    })
}

/// Process a single inbound channel message.
///
/// 1. Truncates overlong messages
/// 2. Sends typing indicator (if supported)
/// 3. Routes to the appropriate agent via bindings, or falls back to first available
/// 4. Runs the agent with a formatted prompt
/// 5. Sends the response back to the originating channel
/// 6. Logs the event
async fn process_message(
    state: &SharedState,
    channel_manager: &Arc<ChannelManager>,
    config: &InboundConfig,
    msg: ChannelMessage,
) -> Result<(), String> {
    let channel_type = msg.source.channel_type().to_string();
    let user_id = msg.source.user_id.clone();
    let chat_id = msg.source.chat_id.clone().unwrap_or_default();

    // ── Fix #5: Message deduplication ──────────────────────────────────
    {
        use std::sync::Mutex;
        static SEEN: std::sync::LazyLock<Mutex<std::collections::VecDeque<String>>> =
            std::sync::LazyLock::new(|| Mutex::new(std::collections::VecDeque::new()));
        let dedup_key = format!("{}:{}:{}", channel_type, chat_id, msg.id);
        if let Ok(mut seen) = SEEN.lock() {
            if seen.contains(&dedup_key) {
                debug!("Duplicate message skipped: {}", dedup_key);
                return Ok(());
            }
            seen.push_back(dedup_key);
            if seen.len() > 500 {
                seen.pop_front();
            }
        }
    }

    // ── Fix #1: Skip bot messages unless this agent is @mentioned ─────
    if msg.source.sender_type.is_bot() {
        let is_mentioned = msg.content.contains("@everyone")
            || msg.content.contains("@here");
        // Check if any agent name from the registry is mentioned
        // Discord mentions use both <@USER_ID> and @name formats
        let agent_mentioned = {
            let st = state.read().await;
            st.agent_registry
                .list()
                .iter()
                .any(|a| {
                    // Check plain text @name mention
                    msg.content.contains(&format!("@{}", a.name))
                    // Check Discord <@id> mention format
                    || msg.content.contains(&format!("<@{}>", a.agent_id))
                    // Check case-insensitive name mention (e.g., @Zeus100, @zeus100)
                    || msg.content.to_lowercase().contains(&format!("@{}", a.name.to_lowercase()))
                })
        };
        if !is_mentioned && !agent_mentioned {
            debug!(
                "Skipping bot message from {}/{} — not mentioned",
                channel_type, user_id
            );
            return Ok(());
        }
    }

    // Truncate overlong messages (UTF-8 safe — previously panicked on
    // multi-byte boundaries when content length exceeded max_message_len).
    let content = if msg.content.len() > config.max_message_len {
        warn!(
            "Truncating inbound message from {}/{} ({} -> {} bytes)",
            channel_type,
            user_id,
            msg.content.len(),
            config.max_message_len
        );
        truncate(&msg.content, config.max_message_len)
    } else {
        msg.content.clone()
    };

    info!(
        "Inbound message from {}/{}/{}: {} chars",
        channel_type,
        user_id,
        chat_id,
        content.len()
    );

    // Start periodic typing indicator (Discord typing expires after 10s,
    // so we re-send every 8s while the agent is cooking a response).
    // The task stops when `typing_done` is set to true after the response
    // is delivered.
    let typing_done = if config.send_typing {
        let done = Arc::new(AtomicBool::new(false));
        let typing_source = msg.source.clone();
        let typing_mgr = channel_manager.clone();
        let done_clone = done.clone();

        tokio::spawn(async move {
            // Send initial typing immediately
            {
                let mgr = &typing_mgr;
                let _ = mgr.send_typing(&typing_source).await;
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(8));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                if done_clone.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {
                        if done_clone.load(Ordering::Relaxed) {
                            break;
                        }
                        let mgr = &typing_mgr;
                        let _ = mgr.send_typing(&typing_source).await;
                    }
                }
            }
        });

        Some(done)
    } else {
        None
    };

    // ── Fix #2: Simple message classification ──────────────────────────
    // Classify message complexity before routing to agent.
    // Silent messages are skipped entirely. Simple messages use a lightweight
    // response path (no tool registration, minimal system prompt).
    let is_dm = msg.source.chat_id.is_none();
    let is_mentioned = {
        let st = state.read().await;
        msg.content.contains("@everyone")
            || msg.content.contains("@here")
            || st.agent_registry.list().iter().any(|a| {
                msg.content.to_lowercase().contains(&a.name.to_lowercase())
            })
    };
    let complexity = classify_message(&content, is_mentioned, is_dm);
    match complexity {
        MessageComplexity::Silent => {
            debug!(
                "Silent classification for {}/{} — skipping LLM call",
                channel_type, user_id
            );
            return Ok(());
        }
        MessageComplexity::Simple | MessageComplexity::Standard => {
            // Both proceed to agent.run() — Simple could use a lighter model
            // in the future, but for now both take the standard path.
        }
    }

    // Route to the appropriate agent
    let (agent_id, agent_arc) = find_agent(state, &channel_type, &user_id, &chat_id).await?;

    // Format prompt with channel context
    let prompt = format_prompt(&channel_type, &user_id, &chat_id, &content);

    // Bridge channel attachments → agent multimodal pipeline.
    // - Images and PDFs → forwarded as zeus_core::Attachment for vision models
    // - Audio (voice messages, audio files) → transcribed via STT (Groq/OpenAI
    //   Whisper) and injected as "[Voice message] <transcript>" into the prompt
    // - Text files (text/* MIME) → decoded as UTF-8 and injected as
    //   "[File: filename] <content>" into the prompt
    // - Other MIMEs (video, archives, etc.) → skipped with debug log
    let ProcessedAttachments {
        visual: core_attachments,
        audio_transcripts,
        text_contents,
    } = process_attachments(&msg.attachments).await;

    // Prepend audio transcripts and text file contents to the prompt so
    // the agent sees them as part of the user message.
    let prompt = {
        let mut prefix_parts: Vec<String> = Vec::new();

        if !audio_transcripts.is_empty() {
            let transcript_block = audio_transcripts
                .iter()
                .map(|t| format!("[Voice message] {}", t))
                .collect::<Vec<_>>()
                .join("\n");
            info!(
                "Injected {} audio transcript(s) into prompt for agent '{}'",
                audio_transcripts.len(),
                agent_id
            );
            prefix_parts.push(transcript_block);
        }

        if !text_contents.is_empty() {
            info!(
                "Injected {} text file(s) into prompt for agent '{}'",
                text_contents.len(),
                agent_id
            );
            prefix_parts.extend(text_contents);
        }

        if prefix_parts.is_empty() {
            prompt
        } else {
            format!("{}\n{}", prefix_parts.join("\n"), prompt)
        }
    };

    // Run the agent (with attachments if any vision-capable files were
    // harvested, otherwise the lighter text-only path).
    let response = {
        let mut agent = agent_arc.write().await;

        // ── Per-channel session routing ──────────────────────────────────
        // Without this, every channel bound to the same agent_id (e.g. the
        // "default" agent) funnels into ONE session file, cross-contaminating
        // conversation history between distinct channels. Derive a
        // deterministic per-channel session id and swap it in so each channel
        // keeps its own history (surviving gateway restarts via resume).
        {
            let key = if user_id.is_empty() {
                zeus_session::ChannelKey::new(channel_type.clone(), chat_id.clone())
            } else {
                zeus_session::ChannelKey::dm(
                    channel_type.clone(),
                    chat_id.clone(),
                    user_id.clone(),
                )
            };
            let session_id = zeus_session::derive_session_id(&key);
            if session_id != agent.session().id {
                if let Some(sessions_dir) = agent.session().path().parent() {
                    let sessions_dir = sessions_dir.to_path_buf();
                    let channel_session =
                        zeus_session::Session::resume_or_create(&sessions_dir, &session_id).await;
                    agent.set_session(channel_session);
                }
            }
        }

        let result = if core_attachments.is_empty() {
            agent.run(&prompt).await
        } else {
            debug!(
                "Routing {} attachment(s) to agent '{}' via run_with_attachments",
                core_attachments.len(),
                agent_id
            );
            agent.run_with_attachments(&prompt, core_attachments).await
        };
        result.map_err(|e| format!("Agent '{}' failed: {}", agent_id, e))?
    };

    info!(
        "Agent '{}' responded to {}/{}: {} chars",
        agent_id,
        channel_type,
        user_id,
        response.len()
    );

    // Stop typing indicator — response is ready
    if let Some(done) = typing_done {
        done.store(true, Ordering::Relaxed);
    }

    // Send response back through the channel
    {
        let mgr = &channel_manager;
        mgr.send(&msg.source, &response)
            .await
            .map_err(|e| format!("Failed to send response via {}: {}", channel_type, e))?;
    }

    // Update activity counters
    {
        let mut st = state.write().await;
        st.agent_registry.update_activity(&agent_id);
    }

    // Log to workspace
    if config.log_to_athena {
        let st = state.read().await;
        let note = format!(
            "[Channel] {}/{}: {} -> agent '{}': {}",
            channel_type,
            user_id,
            truncate(&content, 100),
            agent_id,
            truncate(&response, 100)
        );
        let _ = st.workspace.note(&note).await;
    }

    Ok(())
}

/// Find the appropriate agent for a channel message.
///
/// First tries binding-based routing (AgentRegistry.route()), then
/// falls back to the first available spawned agent.
async fn find_agent(
    state: &SharedState,
    channel_type: &str,
    user_id: &str,
    chat_id: &str,
) -> Result<(String, Arc<RwLock<zeus_agent::Agent>>), String> {
    let st = state.read().await;

    // Try binding-based routing first
    if let Some(instance) = st.agent_registry.route(channel_type, user_id, chat_id) {
        return Ok((instance.agent_id.clone(), instance.agent.clone()));
    }

    // Fallback: first available agent
    if let Some(instance) = st.agent_registry.list().into_iter().next() {
        info!(
            "No binding match for {}/{}/{} — falling back to agent '{}'",
            channel_type, user_id, chat_id, instance.agent_id
        );
        return Ok((instance.agent_id.clone(), instance.agent.clone()));
    }

    Err(format!(
        "No agent available to handle message from {}/{}",
        channel_type, user_id
    ))
}

/// Format an inbound channel message as a prompt for the agent.
/// Message complexity classification for token optimization.
#[derive(Debug, PartialEq)]
enum MessageComplexity {
    /// No response needed — other agents chatting, ACKs, irrelevant chatter
    Silent,
    /// Quick response, no tools needed — greetings, status pings, simple questions
    Simple,
    /// Full agent processing — tasks, code, analysis, multi-step work
    Standard,
}

/// Classify inbound message complexity to avoid burning tokens on trivial messages.
///
/// Rules:
/// - Not mentioned in group chat → Silent (skip entirely)
/// - Very short messages (< 5 words) without task keywords → Simple
/// - Contains code blocks, file paths, or task verbs → Standard
fn classify_message(content: &str, is_mentioned: bool, is_dm: bool) -> MessageComplexity {
    // In group chats, if not mentioned → skip
    if !is_dm && !is_mentioned {
        return MessageComplexity::Silent;
    }

    let word_count = content.split_whitespace().count();
    let has_code = content.contains("```") || content.contains("```");
    let has_path = content.contains('/') && content.contains('.');
    let task_keywords = [
        "fix", "build", "deploy", "implement", "create", "write", "review",
        "audit", "check", "test", "run", "push", "commit", "merge", "debug",
        "spawn", "install", "configure", "update", "delete", "remove", "analyze",
    ];
    let has_task = task_keywords
        .iter()
        .any(|kw| content.to_lowercase().contains(kw));

    // Code, paths, or task keywords → Standard
    if has_code || (has_path && word_count > 3) || (has_task && word_count > 5) {
        return MessageComplexity::Standard;
    }

    // Short messages without complexity signals → Simple
    if word_count < 10 {
        return MessageComplexity::Simple;
    }

    MessageComplexity::Standard
}

fn format_prompt(channel_type: &str, user_id: &str, chat_id: &str, content: &str) -> String {
    if chat_id.is_empty() {
        format!("[{channel_type}] {user_id}: {content}")
    } else {
        format!("[{channel_type}/{chat_id}] {user_id}: {content}")
    }
}

/// Result of processing inbound attachments — visual assets go to the LLM
/// multimodal pipeline, audio gets transcribed to text via STT, text files
/// are decoded as UTF-8 and injected into the prompt.
struct ProcessedAttachments {
    /// Images and PDFs for the LLM vision path.
    visual: Vec<zeus_core::Attachment>,
    /// Transcribed text from audio attachments (voice messages, audio files).
    audio_transcripts: Vec<String>,
    /// Decoded text content from text/* MIME attachments (HTML, CSV, JSON, code, etc.).
    text_contents: Vec<String>,
}

/// Process inbound `ChannelAttachment`s, splitting them by type:
///
/// - **Images** (`image/*`) and **PDFs** (`application/pdf`) → forwarded as
///   `zeus_core::Attachment` for the LLM multimodal pipeline.
/// - **Audio** (`audio/*`) → downloaded, transcribed via Groq or OpenAI
///   Whisper STT, and returned as text strings.
/// - **Text** (`text/*`) → downloaded, decoded as UTF-8, and returned as
///   formatted text strings for prompt injection (capped at 1 MiB).
/// - **Other MIMEs** (video, archives, etc.) → skipped with debug log.
///
/// Download/transcription/decode failures degrade gracefully — a warning is
/// logged and the attachment is skipped rather than failing the whole message.
async fn process_attachments(attachments: &[ChannelAttachment]) -> ProcessedAttachments {
    let mut visual = Vec::new();
    let mut audio_transcripts = Vec::new();
    let mut text_contents = Vec::new();

    for att in attachments {
        let is_image = att.mime_type.starts_with("image/");
        let is_pdf = att.mime_type == "application/pdf";
        let is_audio = att.mime_type.starts_with("audio/");
        let is_text = att.mime_type.starts_with("text/");

        if is_text {
            // Text path: download/read bytes → UTF-8 decode → inject into prompt
            let text = match read_text_attachment(att).await {
                Ok(t) if !t.trim().is_empty() => {
                    info!(
                        "Read text attachment (mime={}, filename={:?}): {} chars",
                        att.mime_type,
                        att.filename,
                        t.len()
                    );
                    t
                }
                Ok(_) => {
                    debug!(
                        "Text attachment decoded to empty content (mime={}, filename={:?})",
                        att.mime_type, att.filename
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        "Failed to read text attachment (mime={}, filename={:?}): {}",
                        att.mime_type, att.filename, e
                    );
                    continue;
                }
            };
            text_contents.push(text);
            continue;
        }

        if is_extractable_doc(&att.mime_type) {
            // Extractable-doc path: download bytes → text extraction → prompt injection.
            // Covers DOCX / PPTX / XLSX / XLS. Mirrors text/* shape via `text_contents`.
            match read_doc_attachment(att).await {
                Ok(t) if !t.trim().is_empty() => {
                    info!(
                        "Extracted text from doc attachment (mime={}, filename={:?}): {} chars",
                        att.mime_type,
                        att.filename,
                        t.len()
                    );
                    text_contents.push(t);
                }
                Ok(_) => {
                    debug!(
                        "Doc attachment extracted to empty content (mime={}, filename={:?})",
                        att.mime_type, att.filename
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to extract text from doc attachment (mime={}, filename={:?}): {}",
                        att.mime_type, att.filename, e
                    );
                }
            }
            continue;
        }

        if is_audio {
            // Audio path: download bytes → STT → text transcript
            match transcribe_audio_attachment(att).await {
                Ok(transcript) if !transcript.trim().is_empty() => {
                    info!(
                        "Transcribed audio attachment (mime={}, filename={:?}): {} chars",
                        att.mime_type,
                        att.filename,
                        transcript.len()
                    );
                    audio_transcripts.push(transcript);
                }
                Ok(_) => {
                    debug!(
                        "Audio attachment transcribed to empty text (mime={}, filename={:?})",
                        att.mime_type, att.filename
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to transcribe audio attachment (mime={}, filename={:?}): {}",
                        att.mime_type, att.filename, e
                    );
                }
            }
            continue;
        }

        if !is_image && !is_pdf {
            debug!(
                "Skipping non-visual/non-audio attachment (mime={}, filename={:?})",
                att.mime_type, att.filename
            );
            continue;
        }

        // Visual path: images and PDFs forwarded to LLM multimodal pipeline.
        let core_att = if let Some(data) = &att.data {
            Some(zeus_core::Attachment::from_data(
                att.mime_type.clone(),
                data.clone(),
            ))
        } else if let Some(url) = &att.url {
            let is_discord_cdn = url.contains("cdn.discordapp.com")
                || url.contains("media.discordapp.net");
            if is_discord_cdn {
                match download_attachment_bytes(url).await {
                    Ok(bytes) => Some(zeus_core::Attachment::from_data(
                        att.mime_type.clone(),
                        bytes,
                    )),
                    Err(e) => {
                        warn!(
                            "Failed to eagerly download Discord CDN attachment \
                             (mime={}, url={}): {} — falling back to URL ref",
                            att.mime_type, url, e
                        );
                        Some(zeus_core::Attachment::from_url(
                            url.clone(),
                            att.mime_type.clone(),
                        ))
                    }
                }
            } else {
                Some(zeus_core::Attachment::from_url(
                    url.clone(),
                    att.mime_type.clone(),
                ))
            }
        } else {
            warn!(
                "Skipping attachment with neither data nor url (mime={}, filename={:?})",
                att.mime_type, att.filename
            );
            None
        };

        if let Some(mut a) = core_att {
            a.filename = att.filename.clone();
            visual.push(a);
        }
    }

    ProcessedAttachments {
        visual,
        audio_transcripts,
        text_contents,
    }
}

/// Maximum size for text/* attachments before truncation (1 MiB).
const MAX_TEXT_ATTACHMENT_BYTES: usize = 1024 * 1024;

/// Read a text/* `ChannelAttachment` and return its UTF-8 content.
///
/// 1. Gets bytes (from inline data or by downloading the URL)
/// 2. Decodes as UTF-8
/// 3. Caps at `MAX_TEXT_ATTACHMENT_BYTES` with a truncation marker
/// 4. Returns formatted string: `[File: filename]\n<content>`
fn read_text_attachment<'a>(att: &'a ChannelAttachment) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
    Box::pin(async move {
        // Step 1: Get bytes
        let bytes = if let Some(data) = &att.data {
            data.clone()
        } else if let Some(url) = &att.url {
            download_attachment_bytes_capped(url, MAX_TEXT_ATTACHMENT_BYTES + 1).await?
        } else {
            return Err("attachment has neither data nor url".to_string());
        };

        // Step 2: Cap + truncation marker
        let (bytes, truncated) = if bytes.len() > MAX_TEXT_ATTACHMENT_BYTES {
            (bytes[..MAX_TEXT_ATTACHMENT_BYTES].to_vec(), true)
        } else {
            (bytes, false)
        };

        // Step 3: UTF-8 decode
        let mut text = String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode: {}", e))?;

        if truncated {
            text.push_str("\n\n[... file truncated at 1 MiB ...]");
        }

        // Step 4: Format with filename header
        let filename = att.filename.as_deref().unwrap_or("unnamed");
        Ok(format!("[File: {}]\n{}", filename, text))
    })
}

/// Maximum size for binary doc attachments before refusing extraction (10 MiB).
///
/// Larger than text/* cap because office docs carry significant binary
/// overhead (zip + XML + media). Extracted text is still capped downstream
/// by the LLM context budget.
const MAX_DOC_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;

/// Read a binary office-doc `ChannelAttachment` and return its extracted text.
///
/// Steps:
/// 1. Get bytes (from inline data or by downloading the URL, capped)
/// 2. Dispatch to the appropriate extractor based on MIME
/// 3. Format with filename header (matches `read_text_attachment` shape)
///
/// Returns `Err` on download / extraction failure — the caller is expected to
/// fail soft (warn + skip), mirroring the audio-transcript path.
fn read_doc_attachment<'a>(
    att: &'a ChannelAttachment,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
    Box::pin(async move {
        // Step 1: Get bytes (capped at 10 MiB to bound memory).
        let bytes = if let Some(data) = &att.data {
            if data.len() > MAX_DOC_ATTACHMENT_BYTES {
                return Err(format!(
                    "doc attachment exceeds {} MiB cap ({} bytes)",
                    MAX_DOC_ATTACHMENT_BYTES / (1024 * 1024),
                    data.len()
                ));
            }
            data.clone()
        } else if let Some(url) = &att.url {
            download_attachment_bytes_capped(url, MAX_DOC_ATTACHMENT_BYTES + 1).await?
        } else {
            return Err("attachment has neither data nor url".to_string());
        };

        if bytes.len() > MAX_DOC_ATTACHMENT_BYTES {
            return Err(format!(
                "doc attachment exceeds {} MiB cap ({} bytes downloaded)",
                MAX_DOC_ATTACHMENT_BYTES / (1024 * 1024),
                bytes.len()
            ));
        }

        // Step 2: Extract via media_extract dispatcher.
        let extracted = extract_text_from_doc(&bytes, &att.mime_type).map_err(|e: ExtractError| {
            format!("text extraction failed: {}", e)
        })?;

        // Step 3: Format with filename header (matches text/* path).
        let filename = att.filename.as_deref().unwrap_or("unnamed");
        Ok(format!("[File: {}]\n{}", filename, extracted))
    })
}

/// Transcribe an audio `ChannelAttachment` to text via Groq or OpenAI Whisper.
///
/// 1. Gets audio bytes (from inline data or by downloading the URL)
/// 2. Sends to STT API as a multipart form upload
/// 3. Returns the transcribed text
///
/// Supports all common audio formats (OGG/Opus, MP3, WAV, M4A, FLAC, etc.)
/// — the Whisper API handles format conversion internally.
async fn transcribe_audio_attachment(att: &ChannelAttachment) -> Result<String, String> {
    // Step 1: Get audio bytes
    let audio_bytes = if let Some(data) = &att.data {
        data.clone()
    } else if let Some(url) = &att.url {
        download_attachment_bytes(url).await?
    } else {
        return Err("attachment has neither data nor url".to_string());
    };

    if audio_bytes.is_empty() {
        return Ok(String::new());
    }

    // Step 2: Select STT provider (Groq preferred, OpenAI fallback)
    let (api_key, endpoint, model) = if let Ok(key) = std::env::var("GROQ_API_KEY") {
        (
            key,
            "https://api.groq.com/openai/v1/audio/transcriptions",
            "whisper-large-v3",
        )
    } else if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        (
            key,
            "https://api.openai.com/v1/audio/transcriptions",
            "whisper-1",
        )
    } else {
        return Err("No STT API key found. Set GROQ_API_KEY or OPENAI_API_KEY.".to_string());
    };

    // Determine filename and MIME for the upload.
    // The Whisper API infers format from the file extension.
    let filename = att
        .filename
        .clone()
        .unwrap_or_else(|| mime_to_filename(&att.mime_type));

    debug!(
        "Transcribing audio: {} bytes, mime={}, filename={}, model={}",
        audio_bytes.len(),
        att.mime_type,
        filename,
        model
    );

    // Step 3: Send to STT API
    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name(filename)
        .mime_str(&att.mime_type)
        .map_err(|e| format!("MIME error: {}", e))?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model.to_string())
        .text("response_format", "json".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("client build: {}", e))?;

    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("STT request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("STT API returned {}: {}", status, body));
    }

    let resp_json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("STT parse error: {}", e))?;

    Ok(resp_json
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

/// Map a MIME type to a sensible filename for the Whisper API upload.
fn mime_to_filename(mime: &str) -> String {
    match mime {
        "audio/ogg" => "voice.ogg".to_string(),
        "audio/mpeg" | "audio/mp3" => "audio.mp3".to_string(),
        "audio/wav" | "audio/x-wav" => "audio.wav".to_string(),
        "audio/flac" => "audio.flac".to_string(),
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "audio.m4a".to_string(),
        "audio/webm" => "audio.webm".to_string(),
        _ => "audio.ogg".to_string(), // Safe default — Whisper handles OGG well
    }
}

/// Legacy wrapper — tests that only care about visual attachments can still
/// call this. Delegates to `process_attachments` and discards audio transcripts.
#[cfg(test)]
async fn convert_attachments(attachments: &[ChannelAttachment]) -> Vec<zeus_core::Attachment> {
    process_attachments(attachments).await.visual
}

/// Download attachment bytes from a URL with a size cap and reasonable timeout.
/// Used for eager fetching of Discord CDN signed URLs.
///
/// Streams the response body chunk-by-chunk, enforcing the size cap during
/// download so a server with a missing or lying Content-Length header cannot
/// force us to buffer an unbounded payload before the check catches it.
async fn download_attachment_bytes(url: &str) -> Result<Vec<u8>, String> {
    download_attachment_bytes_capped(url, MAX_ATTACHMENT_DOWNLOAD_BYTES).await
}

/// Capped streaming downloader. Extracted so tests can exercise the
/// oversized-payload path with a tiny cap instead of needing to produce
/// 32 MiB of mock body data.
async fn download_attachment_bytes_capped(url: &str, cap: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("client build: {}", e))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("send: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    // Fast-fail if the server honestly reports an oversized body via
    // Content-Length — we can reject before reading any bytes.
    if let Some(len) = resp.content_length() {
        if len as usize > cap {
            return Err(format!(
                "attachment too large: {} bytes > {} cap",
                len, cap
            ));
        }
    }

    // Stream chunks and enforce the cap during download. A server that
    // omits Content-Length (or lies about it) cannot force us to buffer
    // an unbounded payload — we bail as soon as the running total
    // exceeds the cap.
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("body: {}", e))?
    {
        if buf.len().saturating_add(chunk.len()) > cap {
            return Err(format!(
                "attachment too large: exceeded {} byte cap during streaming",
                cap
            ));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

/// Truncate a string to a maximum length, appending "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let end = zeus_core::floor_char_boundary(s, max_len);
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeus_channels::{ChannelMessage, ChannelSource};

    #[test]
    fn test_format_prompt_with_chat_id() {
        let prompt = format_prompt("telegram", "user123", "chat456", "Hello bot");
        assert_eq!(prompt, "[telegram/chat456] user123: Hello bot");
    }

    #[test]
    fn test_format_prompt_without_chat_id() {
        let prompt = format_prompt("discord", "user789", "", "Hi there");
        assert_eq!(prompt, "[discord] user789: Hi there");
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let long = "a".repeat(200);
        let result = truncate(&long, 50);
        assert_eq!(result.len(), 53); // 50 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("12345", 5), "12345");
    }

    #[test]
    fn test_inbound_config_defaults() {
        let cfg = InboundConfig::default();
        assert_eq!(cfg.max_message_len, 50_000);
        assert!(cfg.send_typing);
        assert!(cfg.log_to_athena);
    }

    // ── Per-channel session routing contract ─────────────────────────────
    // These lock the derivation used by `process_message` to swap a
    // per-channel session in before `agent.run()`. Two distinct channels on
    // the same agent MUST resolve to distinct session ids (no cross-channel
    // history pollution), and DMs must fold the user_id into the key.

    #[test]
    fn test_inbound_session_routing_distinct_channels() {
        let a = zeus_session::derive_session_id(&zeus_session::ChannelKey::new(
            "discord", "1024483997306339439",
        ));
        let b = zeus_session::derive_session_id(&zeus_session::ChannelKey::new(
            "discord", "1503740515147845752",
        ));
        assert_ne!(a, b, "two discord channels must not share a session");
        assert_eq!(a, "agent:discord:1024483997306339439");
    }

    #[test]
    fn test_inbound_session_routing_group_vs_dm() {
        // Group: no user_id → channel-scoped session shared by participants.
        let group = zeus_session::derive_session_id(&zeus_session::ChannelKey::new(
            "discord", "999",
        ));
        // DM: user_id folded in → per-user session even on same chat_id.
        let dm = zeus_session::derive_session_id(&zeus_session::ChannelKey::dm(
            "discord", "999", "42",
        ));
        assert_eq!(group, "agent:discord:999");
        assert_eq!(dm, "agent:discord:999:42");
        assert_ne!(group, dm);
    }

    #[test]
    fn test_channel_message_construction() {
        let source = ChannelSource::with_chat("telegram", "user1", "chat1");
        let msg = ChannelMessage::new(source.clone(), "Hello from Telegram".to_string());
        assert_eq!(msg.source.channel_type(), "telegram");
        assert_eq!(msg.source.user_id, "user1");
        assert_eq!(msg.source.chat_id, Some("chat1".to_string()));
        assert_eq!(msg.content, "Hello from Telegram");
    }

    #[test]
    fn test_message_truncation_boundary() {
        let config = InboundConfig {
            max_message_len: 10,
            ..Default::default()
        };
        let long_msg = "a".repeat(20);
        let truncated = if long_msg.len() > config.max_message_len {
            long_msg[..zeus_core::floor_char_boundary(&long_msg, config.max_message_len)].to_string()
        } else {
            long_msg.clone()
        };
        assert_eq!(truncated.len(), 10);
    }

    #[test]
    fn test_routing_prompt_includes_metadata() {
        // Verify the prompt format preserves channel context for the agent
        let prompt = format_prompt("slack", "alice", "general", "deploy to prod");
        assert!(prompt.contains("slack"));
        assert!(prompt.contains("alice"));
        assert!(prompt.contains("general"));
        assert!(prompt.contains("deploy to prod"));
    }

    #[test]
    fn test_classify_message_silent_in_group_not_mentioned() {
        assert_eq!(
            classify_message("hey guys whats up", false, false),
            MessageComplexity::Silent
        );
    }

    #[test]
    fn test_classify_message_simple_short_dm() {
        assert_eq!(
            classify_message("ping", true, true),
            MessageComplexity::Simple
        );
        assert_eq!(
            classify_message("status?", true, true),
            MessageComplexity::Simple
        );
    }

    #[test]
    fn test_classify_message_standard_with_task() {
        assert_eq!(
            classify_message("please fix the build error in setup.rs", true, false),
            MessageComplexity::Standard
        );
        assert_eq!(
            classify_message("deploy the latest version to production now", true, true),
            MessageComplexity::Standard
        );
    }

    #[test]
    fn test_classify_message_standard_with_code() {
        assert_eq!(
            classify_message("check this:\n```rust\nfn main() {}\n```", true, true),
            MessageComplexity::Standard
        );
    }

    #[test]
    fn test_classify_message_dm_always_processes() {
        // DM with short message → Simple (not Silent)
        assert_eq!(
            classify_message("hi", false, true),
            MessageComplexity::Simple
        );
    }

    #[tokio::test]
    async fn test_convert_attachments_empty() {
        let out = convert_attachments(&[]).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_convert_attachments_image_inline_data() {
        let att = ChannelAttachment::from_data(vec![0xFF, 0xD8, 0xFF, 0xE0], "image/jpeg")
            .with_filename("test.jpg");
        let out = convert_attachments(&[att]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mime_type, "image/jpeg");
        assert_eq!(out[0].data, vec![0xFF, 0xD8, 0xFF, 0xE0]);
        assert_eq!(out[0].filename.as_deref(), Some("test.jpg"));
        assert!(out[0].has_data());
        assert!(!out[0].is_url_ref());
    }

    #[tokio::test]
    async fn test_convert_attachments_image_non_discord_url_passes_through() {
        // Non-Discord URLs are passed through as URL refs — the LLM
        // provider fetches them server-side.
        let att = ChannelAttachment::from_url(
            "https://example.com/cat.png",
            "image/png",
        );
        let out = convert_attachments(&[att]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mime_type, "image/png");
        assert_eq!(
            out[0].source_url.as_deref(),
            Some("https://example.com/cat.png")
        );
        assert!(out[0].is_url_ref());
    }

    #[tokio::test]
    async fn test_convert_attachments_pdf_inline_data() {
        let att = ChannelAttachment::from_data(b"%PDF-1.7".to_vec(), "application/pdf")
            .with_filename("doc.pdf");
        let out = convert_attachments(&[att]).await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mime_type, "application/pdf");
        assert_eq!(out[0].filename.as_deref(), Some("doc.pdf"));
    }

    #[tokio::test]
    async fn test_convert_attachments_skips_audio() {
        // Audio attachments are skipped at this layer — they require
        // upstream STT before becoming part of the user message.
        let att = ChannelAttachment::from_url(
            "https://example.com/voice.ogg",
            "audio/ogg",
        );
        let out = convert_attachments(&[att]).await;
        assert!(
            out.is_empty(),
            "audio attachments must be skipped at the bridge layer"
        );
    }

    #[tokio::test]
    async fn test_convert_attachments_skips_video_and_other() {
        let video = ChannelAttachment::from_url("https://example.com/clip.mp4", "video/mp4");
        let zip = ChannelAttachment::from_url("https://example.com/archive.zip", "application/zip");
        let out = convert_attachments(&[video, zip]).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_convert_attachments_mixed_keeps_only_visual() {
        let image = ChannelAttachment::from_data(vec![0xFF; 4], "image/png")
            .with_filename("a.png");
        let audio = ChannelAttachment::from_url("https://x/y.ogg", "audio/ogg");
        let pdf = ChannelAttachment::from_data(b"%PDF".to_vec(), "application/pdf");
        let out = convert_attachments(&[image, audio, pdf]).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].mime_type, "image/png");
        assert_eq!(out[1].mime_type, "application/pdf");
    }

    #[tokio::test]
    async fn test_convert_attachments_malformed_neither_data_nor_url() {
        // Both url and data are None — should skip with warning, not panic.
        let att = ChannelAttachment {
            url: None,
            data: None,
            mime_type: "image/jpeg".to_string(),
            filename: None,
        };
        let out = convert_attachments(&[att]).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn test_find_agent_no_agents_returns_error() {
        let config = zeus_core::Config::default();
        let state = Arc::new(RwLock::new(
            crate::AppState::new(config).expect("AppState initialization failed"),
        ));
        let result = find_agent(&state, "telegram", "user1", "chat1").await;
        assert!(result.is_err());
        let err = result.err().expect("should be error");
        assert!(err.contains("No agent available"));
    }

    // ========================================================================
    // Streaming download cap tests — regression coverage for the memory
    // buffering bug zeus106 flagged in the audit of 57a369c8. The prior
    // impl called resp.bytes() which buffers the full body before the
    // post-check, so a server omitting/lying about Content-Length could
    // force us to buffer an arbitrarily large payload. The fixed impl
    // streams via resp.chunk() and enforces the cap during download.
    // ========================================================================

    #[tokio::test]
    async fn test_download_attachment_bytes_streaming_cap_enforced_without_content_length() {
        // Adversarial shape: response has NO Content-Length header, body
        // is delimited by Connection: close. The old resp.bytes() impl
        // would happily buffer the whole body before the post-check.
        // The streaming impl must bail mid-download.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should have addr")
            .port();
        let url = format!("http://127.0.0.1:{}/blob", port);

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut req_buf = vec![0u8; 4096];
                let _ = stream.read(&mut req_buf).await;
                // NO Content-Length. HTTP/1.0-style delimited-by-close body.
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(headers.as_bytes()).await;
                let body = vec![0xABu8; 200];
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            }
        });

        let result = download_attachment_bytes_capped(&url, 100).await;
        assert!(
            result.is_err(),
            "200-byte body with 100-byte cap must be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("too large"),
            "error should indicate size overflow, got: {}",
            err
        );
        assert!(
            err.contains("100"),
            "error should mention the cap value (100), got: {}",
            err
        );
        assert!(
            err.to_lowercase().contains("streaming") || err.contains("cap"),
            "error should indicate the streaming cap path, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_download_attachment_bytes_content_length_precheck_still_works() {
        // Honest server reports an oversized Content-Length. The fast
        // path should reject before reading any body bytes.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should have addr")
            .port();
        let url = format!("http://127.0.0.1:{}/blob", port);

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut req_buf = vec![0u8; 4096];
                let _ = stream.read(&mut req_buf).await;
                // Advertise 500 bytes — exceeds the 100-byte cap we'll
                // pass to the downloader. We never actually have to
                // write the body; the client should reject on headers.
                let headers = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 500\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(headers.as_bytes()).await;
                // Still write some bytes so reqwest doesn't hang waiting
                // for them during the header parse phase.
                let _ = stream.write_all(&vec![0u8; 500]).await;
                let _ = stream.shutdown().await;
            }
        });

        let result = download_attachment_bytes_capped(&url, 100).await;
        assert!(
            result.is_err(),
            "Content-Length 500 with 100-byte cap must be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("500") && err.contains("100"),
            "error should mention both the declared size and the cap, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_download_attachment_bytes_under_cap_succeeds() {
        // Happy path: body is smaller than the cap, download succeeds
        // and returns the full bytes.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should have addr")
            .port();
        let url = format!("http://127.0.0.1:{}/blob", port);

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut req_buf = vec![0u8; 4096];
                let _ = stream.read(&mut req_buf).await;
                let body = vec![0x42u8; 50];
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            }
        });

        let result = download_attachment_bytes_capped(&url, 100).await;
        assert!(
            result.is_ok(),
            "50-byte body with 100-byte cap should succeed, got: {:?}",
            result.err()
        );
        let bytes = result.expect("ok branch");
        assert_eq!(bytes.len(), 50);
        assert!(bytes.iter().all(|b| *b == 0x42));
    }
}
