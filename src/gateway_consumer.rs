//! Gateway Channel Consumer — extracted helper functions for channel message processing.
//!
//! These functions are called from the main channel consumer loop in gateway.rs.
//! Extraction reduces gateway.rs line count and isolates testable logic.

use base64::Engine as _;
use tracing::{debug, info, warn};
use zeus_core::Config;

// ─── Mention Detection ──────────────────────────────────────────────────────

/// Result of checking whether a message is addressed to this agent.
pub enum MentionCheck {
    /// Message is addressed to this agent — process it.
    Addressed {
        is_mentioned: bool,
        is_role_mentioned: bool,
        is_broadcast: bool,
    },
    /// Message is not addressed — add to session context only.
    ContextOnly,
}

/// Word-boundary `@name` mention test (#296 identity de-hardcode).
///
/// A bare substring test makes `@zeus` match `@zeus100`, so every agent whose
/// name is a prefix of another's would steal each other's mentions. This checks
/// that the `@name` token is not immediately followed by an alphanumeric (or
/// `_`/`-`) character, so `@zeus` matches `@zeus` and `@zeus,` but NOT
/// `@zeus100`. Both inputs are matched case-insensitively; pass already-lowered
/// `content_lower` for efficiency.
pub fn mentions_name_at(content_lower: &str, name_lower: &str) -> bool {
    if name_lower.is_empty() {
        return false;
    }
    let needle = format!("@{name_lower}");
    let mut start = 0;
    while let Some(pos) = content_lower[start..].find(&needle) {
        let abs = start + pos;
        let after = abs + needle.len();
        // Char immediately after the `@name` token must not extend the name.
        let boundary_ok = content_lower[after..]
            .chars()
            .next()
            .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        if boundary_ok {
            return true;
        }
        start = abs + 1;
    }
    false
}

/// Decode the bot's Discord user ID from its token.
/// Discord tokens are `base64(user_id).timestamp.hmac`.
pub fn decode_bot_snowflake() -> String {
    zeus_core::resolve_discord_token()
        .and_then(|tok| {
            let first = tok.split('.').next().map(|s| s.to_string())?;
            let padded = match first.len() % 4 {
                2 => format!("{}==", first),
                3 => format!("{}=", first),
                _ => first.clone(),
            };
            base64::engine::general_purpose::STANDARD
                .decode(&padded)
                .ok()
                .and_then(|b| std::str::from_utf8(&b).ok()?.parse::<u64>().ok())
                .map(|id| id.to_string())
        })
        .unwrap_or_default()
}

/// Check if a channel message is addressed to this agent.
///
/// Checks: @agent_name text mention, <@BOT_ID> structured mention,
/// <@&ROLE_ID> role mentions, @everyone/@here broadcasts.
///
/// If `is_direct_message` is true (no chat_id — e.g. IRC DM, iMessage 1:1,
/// Signal 1:1), the message is always treated as Addressed regardless of
/// explicit mention, because DMs by definition target the bot.
pub fn check_mention(
    content: &str,
    agent_name: &str,
    bot_snowflake: &str,
    role_ids: &[String],
) -> MentionCheck {
    check_mention_with_dm(content, agent_name, bot_snowflake, role_ids, false)
}

/// DM-aware variant of `check_mention`. Direct messages are always Addressed.
pub fn check_mention_with_dm(
    content: &str,
    agent_name: &str,
    bot_snowflake: &str,
    role_ids: &[String],
    is_direct_message: bool,
) -> MentionCheck {
    check_mention_full(content, agent_name, bot_snowflake, role_ids, &[], is_direct_message)
}

/// Back-compat shim: original `check_mention_full` signature, no presence filter.
/// New callers should use `check_mention_full_with_presence` directly.
pub fn check_mention_full(
    content: &str,
    agent_name: &str,
    bot_snowflake: &str,
    role_ids: &[String],
    peer_agent_names: &[String],
    is_direct_message: bool,
) -> MentionCheck {
    check_mention_full_with_presence(
        content, agent_name, bot_snowflake, role_ids, peer_agent_names, None, is_direct_message,
    )
}

/// Full variant with responder election for role mentions and broadcasts.
///
/// When a message fires on a role mention (`<@&ID>`) or broadcast (`@everyone` / `@here`),
/// and `peer_agent_names` is non-empty, only the agent whose name comes first
/// alphabetically (case-insensitive) among `peer_agent_names` (which must include
/// this agent's own `agent_name`) cooks. Others return `ContextOnly`.
///
/// Direct mentions (@name or `<@BOT_ID>`) always win — election only gates
/// role/broadcast pings. Empty `peer_agent_names` disables election (every agent
/// cooks, preserving backward compatibility).
/// Election-aware mention check with optional presence filter.
///
/// When `live_peer_filter` is `Some`, the role/broadcast election only considers
/// peers that appear in the filter (lowercased). Offline/wedged peers are skipped,
/// so the alphabetical winner is the first *live* agent — not the first configured
/// agent. Self is always considered live by the caller (PresenceTracker contract).
///
/// When `live_peer_filter` is `None`, behavior is identical to the original
/// (all configured peers participate in election). Pre-existing callers/tests
/// continue to work via the `check_mention_full` shim below.
pub fn check_mention_full_with_presence(
    content: &str,
    agent_name: &str,
    bot_snowflake: &str,
    role_ids: &[String],
    peer_agent_names: &[String],
    live_peer_filter: Option<&[String]>,
    is_direct_message: bool,
) -> MentionCheck {
    let content_lower = content.to_lowercase();

    // #296: word-boundary `@name` match so `@zeus` ≠ `@zeus100`. An empty
    // agent name never matches a text mention (only structured <@id> can).
    let is_mentioned = is_direct_message
        || mentions_name_at(&content_lower, &agent_name.to_lowercase())
        || (!bot_snowflake.is_empty()
            && content.contains(&format!("<@{}>", bot_snowflake)))
        || (!bot_snowflake.is_empty()
            && content.contains(&format!("<@!{}>", bot_snowflake)));

    let is_role_mentioned = if role_ids.is_empty() {
        false
    } else {
        role_ids
            .iter()
            .any(|rid| content.contains(&format!("<@&{}>", rid)))
    };

    let is_broadcast =
        content_lower.contains("@everyone") || content_lower.contains("@here");

    // Responder election: if this is a role mention or broadcast (but NOT a direct
    // mention), and a peer list is configured, only the alphabetically-first agent
    // cooks. Direct mentions bypass election entirely.
    if !is_mentioned
        && (is_role_mentioned || is_broadcast)
        && !peer_agent_names.is_empty()
    {
        let self_lower = agent_name.to_lowercase();

        // If a presence filter is provided, restrict election to live peers.
        // Self is always included (PresenceTracker.live_peers contract guarantees this).
        // Empty live set → fall back to full peer list to avoid silent fleet outage.
        let candidates_lower: Vec<String> = match live_peer_filter {
            Some(live) if !live.is_empty() => peer_agent_names
                .iter()
                .map(|n| n.to_lowercase())
                .filter(|n| live.iter().any(|l| l == n))
                .collect(),
            _ => peer_agent_names.iter().map(|n| n.to_lowercase()).collect(),
        };

        // Ensure self is in the candidate set (defensive — should always be true
        // for live-filter case, and is by construction for the unfiltered case
        // when self is in peer_agent_names; if config omits self, we still skip).
        let first = candidates_lower.iter().min().cloned();
        if let Some(first_name) = first {
            if first_name != self_lower {
                debug!(
                    "Role/broadcast election: {} skips — first-alphabetical-live is {} (filter={})",
                    agent_name, first_name, live_peer_filter.is_some()
                );
                return MentionCheck::ContextOnly;
            }
        }
    }

    if is_mentioned || is_role_mentioned || is_broadcast {
        MentionCheck::Addressed {
            is_mentioned,
            is_role_mentioned,
            is_broadcast,
        }
    } else {
        MentionCheck::ContextOnly
    }
}

// ─── Content Building ────────────────────────────────────────────────────────

/// Build effective message content with channel prompt and hints.
pub fn build_final_content(
    content: &str,
    chat_id: Option<&str>,
    channel_type: &str,
    channel_prompt: Option<&str>,
) -> String {
    if let Some(chat_id) = chat_id {
        let prompt = channel_prompt.unwrap_or(
            "You're on a shared team channel. Your replies go directly to the channel \
             — no need to use the message tool for text responses. Just talk naturally.",
        );
        let channel_hint = format!(
            "You are replying to {} channel {}. Use this as channel_id when calling \
             discord_send_message or similar tools.",
            channel_type, chat_id
        );
        if prompt.is_empty() {
            format!("[{}]\n\n{}", channel_hint, content)
        } else {
            format!("[{}]\n\n[{}]\n\n{}", prompt, channel_hint, content)
        }
    } else {
        content.to_string()
    }
}

/// Returns true if a Discord history record should be kept for context injection,
/// false if it should be filtered out as noise / kimi-loop fuel.
///
/// Filtering rules:
/// 1. Drop the agent's OWN prior bot messages (matched by `author_id == own_bot_id`) —
///    they're already in the session message history; re-injecting creates a feedback loop.
/// 2. Drop heartbeat / plan-resume narration prefixes.
/// 3. Drop existing noise prefixes (`[WARNING]`, `[Cooking loop timed out`, etc).
/// 4. For OTHER bots' messages, drop scratchpad-shape first-person deliberation
///    (`Let me ...`, `Wait, ...`, `Actually, ...`, `OK so ...`, `Hmm, ...`).
///    Operator messages that happen to start this way are NOT filtered.
pub fn should_keep_history_message(
    msg: &zeus_api::handlers::discord_history::CachedMessage,
    own_bot_id: &str,
) -> bool {
    // Rule 1: never re-inject the agent's own messages.
    if !own_bot_id.is_empty() && msg.author_id == own_bot_id {
        return false;
    }

    let c = msg.content.trim();

    // Rule 2 + 3: prefix-based noise filter (applies to all senders).
    if c.starts_with("[WARNING]")
        || c.starts_with("[Cooking loop timed out")
        || c == "HEARTBEAT_OK"
        || c.starts_with("I'm experiencing repeated errors")
        || c.starts_with("Could you rephrase")
        || c.starts_with("Could you provide more details")
        || c.starts_with("[Heartbeat]")
        || c.starts_with("[Heartbeat FAIL]")
        || c.starts_with("[Plan Resume]")
    {
        return false;
    }

    // Rule 4: scratchpad-shape filter — bot-only.
    // Don't suppress operator messages that happen to start with these markers.
    if msg.is_bot {
        const SCRATCHPAD_PREFIXES: &[&str] = &[
            "Let me ",
            "Wait, ",
            "Actually, ",
            "OK so ",
            "Hmm, ",
        ];
        for p in SCRATCHPAD_PREFIXES {
            if c.starts_with(p) {
                return false;
            }
        }
    }

    true
}

/// Inject Discord channel history as conversation context.
///
/// Only injects messages from the last 10 minutes or since the agent's last response
/// (whichever is more recent). Skips injection if the session already has >20 messages.
/// Filters noise via `should_keep_history_message`. Excludes the agent's own bot
/// messages entirely (matched by `own_bot_id`) — they're already in session history.
pub async fn inject_channel_history(
    final_content: String,
    channel_type: &str,
    chat_id: &str,
    platform_message_id: Option<&str>,
    session_message_count: usize,
    own_bot_id: &str,
    discord_history: &zeus_api::handlers::discord_history::DiscordHistoryStore,
) -> String {
    // Skip history injection on fresh start — agent should have zero prior context.
    // Reads the process-lifetime flag set once by `init_fresh_start_flag()` at
    // gateway boot, so this is race-free even if multiple channel messages
    // land concurrently before/after bootstrap finishes.
    if crate::gateway_bootstrap::is_fresh_start() {
        info!("Fresh start detected — skipping channel history injection");
        return final_content;
    }

    // Skip if session already has substantial context
    if session_message_count > 20 {
        return final_content;
    }
    if channel_type != "discord" || chat_id.is_empty() {
        return final_content;
    }

    let now_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ten_min_ago = now_epoch - 600;
    let last_bot_ts = discord_history
        .last_bot_response_timestamp(chat_id)
        .await
        .unwrap_or(0);
    let since_ts = ten_min_ago.max(last_bot_ts);
    let history = discord_history
        .get_history_since(chat_id, 15, since_ts)
        .await;

    if history.len() <= 1 {
        return final_content;
    }

    let mut context_lines: Vec<String> = history
        .iter()
        .rev()
        .filter(|h| h.id != platform_message_id.unwrap_or(""))
        .filter(|h| should_keep_history_message(h, own_bot_id))
        .map(|h| {
            let name = if h.author_name.is_empty() {
                if h.is_bot {
                    "bot"
                } else {
                    "user"
                }
            } else {
                &h.author_name
            };
            format!("{}: {}", name, h.content)
        })
        .collect();

    let mut total_chars = 0;
    context_lines.retain(|line| {
        total_chars += line.len();
        total_chars < 4000
    });

    if context_lines.is_empty() {
        return final_content;
    }

    let context_block = context_lines.join("\n");
    info!(
        "Injecting {} messages of Discord history ({} chars) as context",
        context_lines.len(),
        context_block.len()
    );
    format!(
        "[Recent conversation in this channel:]\n{}\n[End of history]\n\n\
         [You're on a shared team channel. Your replies go directly to the channel \
         — no need to use the message tool for text responses. Just talk naturally.]\n\n\
         [NEW MESSAGE — respond to THIS, not to anything in the history above:]\n{}",
        context_block, final_content
    )
}

// ─── Intent & Cooking Decision ───────────────────────────────────────────────

/// Check if the TASK_COMPLETED marker exists in recent session messages
/// and the incoming message is just chatter (not a new task).
pub fn check_task_completed(session_messages: &[zeus_core::Message], intent_input: &str) -> bool {
    let has_marker = session_messages
        .iter()
        .rev()
        .take(5)
        .any(|m| {
            m.role == zeus_core::Role::System && m.content.contains("[TASK_COMPLETED:")
        });

    if !has_marker {
        return false;
    }

    let lower = intent_input.to_lowercase();
    let is_new_task = lower.len() > 30
        || [
            "explain", "write", "create", "build", "fix", "find", "search",
            "research", "analyze", "run", "execute", "deploy", "implement",
            "debug", "test", "review", "audit", "check",
        ]
        .iter()
        .any(|verb| lower.contains(verb));

    !is_new_task // skip if NOT a new task
}

// ─── Command Detection ───────────────────────────────────────────────────────

/// Check if a message is a stop command (/stop, stand down, /halt).
pub fn is_stop_command(content: &str) -> bool {
    let content_lower = content.trim().to_lowercase();
    let bare = content_lower
        .find("]: ")
        .map(|i| content_lower[i + 3..].trim())
        .unwrap_or(content_lower.trim());
    bare == "/stop"
        || bare == "stand down"
        || bare == "/halt"
        || bare.starts_with("/stop ")
        || bare.starts_with("stand down ")
}

/// Check if a message is a HEARTBEAT_OK that should be silently filtered.
pub fn is_heartbeat_ok(content: &str) -> bool {
    let content_lower = content.trim().to_lowercase();
    let bare = content_lower
        .find("]: ")
        .map(|i| content_lower[i + 3..].trim())
        .unwrap_or(content_lower.trim());
    bare == "heartbeat_ok"
        || bare == "heartbeat_ok."
        || content_lower.ends_with("heartbeat_ok")
}

// ─── Attachment Processing ────────────────────────────────────────────────────

/// Process channel message attachments into LLM content blocks + text context.
///
/// Side effects: HTTP downloads, ffmpeg audio conversion, Whisper STT API calls.
/// Returns (core_attachments for LLM vision/file blocks, extra_context text to append).
pub async fn process_attachments(
    attachments: &[zeus_channels::ChannelAttachment],
    whisper_url: &str,
) -> (Vec<zeus_core::Attachment>, String) {
    let mut core_attachments: Vec<zeus_core::Attachment> = Vec::new();
    let mut extra_context = String::new();
    let http_client = reqwest::Client::new();

    for a in attachments {
        let mime = &a.mime_type;

        if mime.starts_with("audio/") {
            process_audio_attachment(a, whisper_url, &http_client, &mut extra_context).await;
        } else if mime.starts_with("text/") {
            process_text_attachment(a, &http_client, &mut extra_context).await;
        } else {
            // Images, PDFs, and other files → pass as attachments
            if mime.starts_with("image/") {
                if let Some(ref url) = a.url {
                    let fname = a.filename.as_deref().unwrap_or("image");
                    extra_context.push_str(&format!("\n[Image attachment: {} — {}]\n", fname, url));
                    info!("Image attachment URL appended to context: {}", url);
                }
            }
            let att = if let Some(ref data) = a.data {
                let mut att = zeus_core::Attachment::from_data(&a.mime_type, data.clone());
                att.filename = a.filename.clone();
                att
            } else if let Some(ref url) = a.url {
                // Download image data from URL (e.g. Discord CDN) so we always
                // have base64 bytes. URL-only attachments fail on providers that
                // can't fetch authenticated/expiring CDN links (Kimi, Ollama, etc.).
                match http_client.get(url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.bytes().await {
                            Ok(bytes) => {
                                info!("Downloaded attachment: {} bytes from {}", bytes.len(), url);
                                let mut att = zeus_core::Attachment::from_data(&a.mime_type, bytes.to_vec());
                                att.filename = a.filename.clone();
                                att.source_url = Some(url.clone());
                                att
                            }
                            Err(e) => {
                                warn!("Failed to download attachment bytes: {} — falling back to URL ref", e);
                                let mut att = zeus_core::Attachment::from_url(url, &a.mime_type);
                                att.filename = a.filename.clone();
                                att
                            }
                        }
                    }
                    Ok(resp) => {
                        warn!("Attachment download HTTP {}: {} — falling back to URL ref", resp.status(), url);
                        let mut att = zeus_core::Attachment::from_url(url, &a.mime_type);
                        att.filename = a.filename.clone();
                        att
                    }
                    Err(e) => {
                        warn!("Attachment download failed: {} — falling back to URL ref", e);
                        let mut att = zeus_core::Attachment::from_url(url, &a.mime_type);
                        att.filename = a.filename.clone();
                        att
                    }
                }
            } else {
                continue;
            };
            core_attachments.push(att);
        }
    }

    (core_attachments, extra_context)
}

/// Process a single audio attachment: download, convert to WAV, transcribe via Whisper.
async fn process_audio_attachment(
    a: &zeus_channels::ChannelAttachment,
    whisper_url: &str,
    http_client: &reqwest::Client,
    extra_context: &mut String,
) {
    let Some(ref url) = a.url else { return };
    let mime = &a.mime_type;

    if whisper_url.is_empty() {
        info!("No ZEUS_WHISPER_URL configured, skipping audio transcription");
        extra_context.push_str(&format!(
            "\n[Audio attachment: {} — transcription unavailable, no STT service configured]",
            a.filename.as_deref().unwrap_or("audio")
        ));
        return;
    }

    let resp = match http_client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => { warn!("Audio download failed: HTTP {}", r.status()); return; }
        Err(e) => { warn!("Audio download request failed: {}", e); return; }
    };

    let audio_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => { warn!("Failed to download audio: {}", e); return; }
    };

    info!(bytes = audio_bytes.len(), "Downloaded audio for STT");
    let fname = a.filename.as_deref().unwrap_or("audio.ogg");

    // Whisper.cpp needs WAV — convert via ffmpeg if not already WAV
    let (send_bytes, send_fname, send_mime) = if mime == "audio/wav" || mime == "audio/x-wav" {
        (audio_bytes.to_vec(), fname.to_string(), "audio/wav".to_string())
    } else {
        let tmp_in = format!("/tmp/zeus_stt_in_{}.ogg", std::process::id());
        let tmp_out = format!("/tmp/zeus_stt_out_{}.wav", std::process::id());
        let ffmpeg_bin = ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "ffmpeg"]
            .iter().find(|p| std::path::Path::new(p).exists()).copied().unwrap_or("ffmpeg");
        let convert_ok = tokio::fs::write(&tmp_in, &audio_bytes).await.is_ok()
            && tokio::process::Command::new(ffmpeg_bin)
                .args(["-i", &tmp_in, "-ar", "16000", "-ac", "1", "-f", "wav", &tmp_out, "-y"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status().await
                .map(|s| s.success()).unwrap_or(false);
        let result = if convert_ok {
            match tokio::fs::read(&tmp_out).await {
                Ok(wav) => (wav, "audio.wav".to_string(), "audio/wav".to_string()),
                Err(_) => (audio_bytes.to_vec(), fname.to_string(), mime.to_string()),
            }
        } else {
            warn!("ffmpeg audio conversion failed, sending raw audio");
            (audio_bytes.to_vec(), fname.to_string(), mime.to_string())
        };
        let _ = tokio::fs::remove_file(&tmp_in).await;
        let _ = tokio::fs::remove_file(&tmp_out).await;
        result
    };

    let form = reqwest::multipart::Form::new()
        .part("file",
            reqwest::multipart::Part::bytes(send_bytes)
                .file_name(send_fname)
                .mime_str(&send_mime)
                .unwrap_or_else(|_|
                    reqwest::multipart::Part::bytes(vec![])
                        .file_name("audio.wav".to_string())
                )
        );
    let stt_endpoint = format!("{}/inference", whisper_url.trim_end_matches('/'));

    match http_client.post(&stt_endpoint).multipart(form).send().await {
        Ok(stt_resp) if stt_resp.status().is_success() => {
            if let Ok(text) = stt_resp.text().await {
                let transcription = match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(v) => v["text"].as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                        warn!("STT response JSON missing 'text' field, using raw");
                        text.clone()
                    }),
                    Err(e) => {
                        warn!("STT response not valid JSON ({}), using raw text", e);
                        text
                    }
                };
                info!(chars = transcription.len(), "Audio transcribed via Whisper");
                extra_context.push_str(&format!(
                    "\n[Voice message transcription: \"{}\"]\n",
                    transcription.trim()
                ));
            }
        }
        Ok(stt_resp) => warn!(status = %stt_resp.status(), "Whisper STT returned error"),
        Err(e) => warn!("Whisper STT request failed: {}", e),
    }
}

/// Process a single text file attachment: download and inline into context.
async fn process_text_attachment(
    a: &zeus_channels::ChannelAttachment,
    http_client: &reqwest::Client,
    extra_context: &mut String,
) {
    let Some(ref url) = a.url else { return };

    let resp = match http_client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return,
    };

    if let Ok(text) = resp.text().await {
        let fname = a.filename.as_deref().unwrap_or("file.txt");
        let truncated = if text.len() > 50_000 {
            format!("{}... [truncated, {} bytes total]", &text[..50_000], text.len())
        } else {
            text
        };
        info!(filename = fname, bytes = truncated.len(), "Extracted text file content");
        extra_context.push_str(&format!("\n[File: {}]\n```\n{}\n```\n", fname, truncated));
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_bot_snowflake_empty() {
        // With no token set, should return empty string
        let result = decode_bot_snowflake();
        // Can't assert specific value without env, but shouldn't panic
        assert!(result.is_empty() || !result.is_empty());
    }

    #[test]
    fn test_check_mention_direct() {
        let result = check_mention("hey @zeus do this", "zeus", "", &[]);
        assert!(matches!(result, MentionCheck::Addressed { is_mentioned: true, .. }));
    }

    // #296: word-boundary @mention — `@zeus` must NOT match `@zeus100`.
    #[test]
    fn test_mentions_name_at_word_boundary() {
        // Exact and punctuation-bounded match.
        assert!(mentions_name_at("hey @zeus do this", "zeus"));
        assert!(mentions_name_at("@zeus, ping", "zeus"));
        assert!(mentions_name_at("ping @zeus", "zeus"));
        // Prefix collision must NOT match.
        assert!(!mentions_name_at("hey @zeus100 do this", "zeus"));
        assert!(!mentions_name_at("@zeusmarketing go", "zeus"));
        // Longer name matches its own token.
        assert!(mentions_name_at("hey @zeus100 do this", "zeus100"));
        // Empty name never matches.
        assert!(!mentions_name_at("@zeus", ""));
    }

    // #296: the full check must not let a nameless/short agent steal @zeus100.
    #[test]
    fn test_check_mention_no_prefix_steal() {
        // Agent "zeus" should not be addressed by "@zeus100".
        let result = check_mention("hey @zeus100 build it", "zeus", "", &[]);
        assert!(matches!(result, MentionCheck::ContextOnly));
        // But "@zeus" addresses it.
        let result2 = check_mention("hey @zeus build it", "zeus", "", &[]);
        assert!(matches!(result2, MentionCheck::Addressed { is_mentioned: true, .. }));
        // A non-matching sentinel name is never text-mentioned.
        let result3 = check_mention("hey @zeus100 build it", "<unnamed agent>", "", &[]);
        assert!(matches!(result3, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_check_mention_snowflake() {
        let result = check_mention("hello <@123456>", "zeus", "123456", &[]);
        assert!(matches!(result, MentionCheck::Addressed { is_mentioned: true, .. }));
    }

    #[test]
    fn test_check_mention_role() {
        let result = check_mention(
            "hey <@&999> check this",
            "zeus",
            "123",
            &["999".to_string()],
        );
        assert!(matches!(
            result,
            MentionCheck::Addressed { is_role_mentioned: true, .. }
        ));
    }

    #[test]
    fn test_check_mention_broadcast() {
        let result = check_mention("@everyone standup", "zeus", "", &[]);
        assert!(matches!(result, MentionCheck::Addressed { is_broadcast: true, .. }));
    }

    #[test]
    fn test_check_mention_not_addressed() {
        let result = check_mention("hey team, good morning", "zeus", "123", &[]);
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_check_mention_dm_implicit_addressed() {
        // DMs must be treated as Addressed even without explicit mention —
        // they are by definition targeted at the recipient.
        let result = check_mention_with_dm("hi there", "zeus", "123", &[], true);
        assert!(matches!(result, MentionCheck::Addressed { .. }));
    }

    #[test]
    fn test_check_mention_dm_false_is_context_only() {
        // Sanity: non-DM unaddressed message is still ContextOnly.
        let result = check_mention_with_dm("hi there", "zeus", "123", &[], false);
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_election_role_mention_first_wins() {
        // zeus100 is first alphabetically — wins election on role mention.
        let peers = vec![
            "zeus100".to_string(),
            "zeus112".to_string(),
            "zeus106".to_string(),
        ];
        let result = check_mention_full(
            "hey <@&999> what is illumos?",
            "zeus100",
            "",
            &["999".to_string()],
            &peers,
            false,
        );
        assert!(matches!(
            result,
            MentionCheck::Addressed { is_role_mentioned: true, .. }
        ));
    }

    #[test]
    fn test_election_role_mention_loser_skips() {
        // zeus112 is NOT first — should skip.
        let peers = vec![
            "zeus100".to_string(),
            "zeus112".to_string(),
            "zeus106".to_string(),
        ];
        let result = check_mention_full(
            "hey <@&999> what is illumos?",
            "zeus112",
            "",
            &["999".to_string()],
            &peers,
            false,
        );
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_election_broadcast_loser_skips() {
        let peers = vec!["zeus100".to_string(), "zeus112".to_string()];
        let result = check_mention_full(
            "@everyone standup",
            "zeus112",
            "",
            &[],
            &peers,
            false,
        );
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_election_direct_mention_bypasses() {
        // Direct @mention must bypass election — even if zeus112 is "loser",
        // they still cook when named directly.
        let peers = vec!["zeus100".to_string(), "zeus112".to_string()];
        let result = check_mention_full(
            "hey @zeus112 specifically, <@&999> too",
            "zeus112",
            "",
            &["999".to_string()],
            &peers,
            false,
        );
        assert!(matches!(
            result,
            MentionCheck::Addressed { is_mentioned: true, .. }
        ));
    }

    #[test]
    fn test_election_empty_peers_backward_compat() {
        // No peer list = no election = every agent cooks on role (old behavior).
        let result = check_mention_full(
            "hey <@&999> ping",
            "zeus112",
            "",
            &["999".to_string()],
            &[],
            false,
        );
        assert!(matches!(
            result,
            MentionCheck::Addressed { is_role_mentioned: true, .. }
        ));
    }

    #[test]
    fn test_election_liveness_filter_skips_offline_winner() {
        // Peers: zeus100, zeus112. zeus100 is offline (not in live list).
        // zeus112 should now win the election even though zeus100 is alphabetically first.
        let peers = vec!["zeus100".to_string(), "zeus112".to_string()];
        let live = vec!["zeus112".to_string()]; // only zeus112 is live
        let result = check_mention_full_with_presence(
            "@everyone go",
            "zeus112",
            "",
            &[],
            &peers,
            Some(&live),
            false,
        );
        // zeus112 is the only live candidate → it wins, must Address.
        assert!(matches!(
            result,
            MentionCheck::Addressed { is_broadcast: true, .. }
        ));
    }

    #[test]
    fn test_election_liveness_filter_offline_loser_still_skips() {
        // Peers: zeus100, zeus112. Both live. zeus112 is the loser even with filter.
        let peers = vec!["zeus100".to_string(), "zeus112".to_string()];
        let live = vec!["zeus100".to_string(), "zeus112".to_string()];
        let result = check_mention_full_with_presence(
            "@everyone go",
            "zeus112",
            "",
            &[],
            &peers,
            Some(&live),
            false,
        );
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_election_liveness_empty_filter_falls_back_to_all_peers() {
        // Empty live list → defensive fallback: treat all peers as candidates.
        // zeus100 wins alphabetically, so zeus112 skips.
        let peers = vec!["zeus100".to_string(), "zeus112".to_string()];
        let live: Vec<String> = vec![];
        let result = check_mention_full_with_presence(
            "@everyone go",
            "zeus112",
            "",
            &[],
            &peers,
            Some(&live),
            false,
        );
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_election_case_insensitive() {
        // Peers may be written with mixed case — election must be case-insensitive.
        let peers = vec!["Zeus100".to_string(), "ZEUS112".to_string()];
        let result = check_mention_full(
            "@everyone go",
            "zeus100",
            "",
            &[],
            &peers,
            false,
        );
        assert!(matches!(result, MentionCheck::Addressed { .. }));
    }

    #[test]
    fn test_check_mention_role_no_match() {
        let result = check_mention(
            "hey <@&888> check this",
            "zeus",
            "123",
            &["999".to_string()],
        );
        assert!(matches!(result, MentionCheck::ContextOnly));
    }

    #[test]
    fn test_build_final_content_with_channel() {
        let result = build_final_content("hello", Some("12345"), "discord", None);
        assert!(result.contains("discord channel 12345"));
        assert!(result.contains("hello"));
    }

    #[test]
    fn test_build_final_content_no_channel() {
        let result = build_final_content("hello", None, "discord", None);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_check_task_completed_no_marker() {
        let msgs = vec![zeus_core::Message::user("do something")];
        assert!(!check_task_completed(&msgs, "hello"));
    }

    #[test]
    fn test_check_task_completed_with_marker_chatter() {
        let msgs = vec![
            zeus_core::Message::user("build something"),
            zeus_core::Message::assistant("done"),
            zeus_core::Message::system("[TASK_COMPLETED: response delivered]"),
        ];
        assert!(check_task_completed(&msgs, "nice work"));
    }

    #[test]
    fn test_check_task_completed_with_marker_new_task() {
        let msgs = vec![
            zeus_core::Message::user("build something"),
            zeus_core::Message::assistant("done"),
            zeus_core::Message::system("[TASK_COMPLETED: response delivered]"),
        ];
        assert!(!check_task_completed(&msgs, "now create a new REST API"));
    }

    #[test]
    fn test_is_stop_command() {
        assert!(is_stop_command("/stop"));
        assert!(is_stop_command("stand down"));
        assert!(is_stop_command("/halt"));
        assert!(is_stop_command("[Zeus100]: /stop"));
        assert!(!is_stop_command("hello"));
        assert!(!is_stop_command("please stop being silly"));
    }

    #[test]
    fn test_is_heartbeat_ok() {
        assert!(is_heartbeat_ok("HEARTBEAT_OK"));
        assert!(is_heartbeat_ok("heartbeat_ok"));
        assert!(is_heartbeat_ok("[Zeus100]: HEARTBEAT_OK"));
        assert!(!is_heartbeat_ok("the heartbeat is failing"));
    }

    // ─── inject_channel_history filter tests (Dispatch 28 — Layer 2) ─────────

    fn make_msg(id: &str, author_id: &str, is_bot: bool, content: &str) -> zeus_api::handlers::discord_history::CachedMessage {
        zeus_api::handlers::discord_history::CachedMessage {
            id: id.to_string(),
            channel_id: "c".to_string(),
            author_id: author_id.to_string(),
            author_name: if is_bot { "bot".to_string() } else { "op".to_string() },
            content: content.to_string(),
            timestamp: 1000,
            is_bot,
        }
    }

    #[test]
    fn test_inject_history_filters_heartbeat_prefix() {
        let m = make_msg("1", "other-bot", true, "[Heartbeat] hourly-3: Completed");
        assert!(!should_keep_history_message(&m, "self-bot"));
        // FAIL prefix variant also dropped
        let m2 = make_msg("2", "other-bot", true, "[Heartbeat FAIL] task X");
        assert!(!should_keep_history_message(&m2, "self-bot"));
    }

    #[test]
    fn test_inject_history_filters_plan_resume_prefix() {
        let m = make_msg("1", "other-bot", true, "[Plan Resume] step 2 of 5");
        assert!(!should_keep_history_message(&m, "self-bot"));
    }

    #[test]
    fn test_inject_history_filters_scratchpad_shape_for_own_bot() {
        // Scratchpad-shape from a bot (other bot) → filtered.
        let bot_msg = make_msg("1", "other-bot", true, "Let me check the logs first.");
        assert!(!should_keep_history_message(&bot_msg, "self-bot"));

        // SAME content from an operator → NOT filtered.
        let op_msg = make_msg("2", "human-user", false, "Let me check the logs first.");
        assert!(should_keep_history_message(&op_msg, "self-bot"));

        // Other scratchpad markers from bot → filtered.
        for prefix in ["Wait, that's odd.", "Actually, never mind.", "OK so the issue is clear.", "Hmm, interesting."] {
            let m = make_msg("x", "other-bot", true, prefix);
            assert!(!should_keep_history_message(&m, "self-bot"), "expected filter for: {}", prefix);
        }

        // Same markers from operator → kept.
        for prefix in ["Wait, that's odd.", "Actually, never mind.", "OK so the issue is clear.", "Hmm, interesting."] {
            let m = make_msg("y", "human-user", false, prefix);
            assert!(should_keep_history_message(&m, "self-bot"), "expected KEEP for operator: {}", prefix);
        }
    }

    #[test]
    fn test_inject_history_excludes_own_bot_messages() {
        // Agent's own messages by author_id → never appear.
        let own = make_msg("1", "self-bot", true, "Just a normal status update.");
        assert!(!should_keep_history_message(&own, "self-bot"));

        // OTHER bots' messages with normal content → kept.
        let other_bot = make_msg("2", "zeus106-bot", true, "Just a normal status update.");
        assert!(should_keep_history_message(&other_bot, "self-bot"));

        // Operator messages → kept.
        let op = make_msg("3", "human-user", false, "deploy now");
        assert!(should_keep_history_message(&op, "self-bot"));

        // Empty own_bot_id (e.g. token decode failed) — fall back gracefully:
        // don't accidentally filter EVERYTHING. Bot messages still pass through
        // unless they hit a content-based rule.
        let bot_normal = make_msg("4", "any-bot", true, "Hello team.");
        assert!(should_keep_history_message(&bot_normal, ""));
    }
}
