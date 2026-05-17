//! Gateway Relay Setup — channel relay initialization for all messaging platforms.
//!
//! Each relay follows the pattern: build config → create adapter → wire inbox callback → start.
//! All relays feed messages into the unified agent inbox for sequential processing.

use std::sync::Arc;
use anyhow::Result;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};
use zeus_core::Config;

/// Start the Telegram relay (Bot API polling → agent inbox).
pub async fn start_telegram_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    enable_agent_processing: bool,
) {
    let Some(ref relay_config) = config.telegram_relay else { return };

    let stt_provider = zeus_channels::telegram_voice::SttProvider::from_config(config);
    let tts_provider = zeus_channels::telegram_voice::TtsProvider::from_config(config);
    let tg_relay_config = zeus_channels::TelegramRelayConfig {
        bot_token: relay_config.bot_token.clone(),
        chat_id: relay_config.chat_id.clone(),
        allowed_users: relay_config.allowed_users.clone(),
        target_session: relay_config.target_session.clone(),
        max_message_length: 4000,
        rate_limit_per_minute: 30,
        enable_groups: true,
        require_mention_in_groups: relay_config.require_mention_in_groups,
        bot_username: None,
        policy: relay_config.policy.clone(),
        webhook_port: None,
        webhook_path: "/telegram/webhook".to_string(),
        webhook_url: None,
        allow_bots: relay_config.allow_bots.clone(),
        fleet_bot_ids: relay_config.fleet_bot_ids.clone(),
        stt_provider,
        tts_provider,
    };
    let relay = zeus_channels::TelegramRelay::new(tg_relay_config);

    if enable_agent_processing {
        let inbox_for_tg = agent_inbox.clone();
        relay.set_message_callback(move |msg: String| {
            let inbox = inbox_for_tg.clone();
            tokio::spawn(async move {
                match inbox.send_and_wait(msg, None, vec![], 1800, false, None).await {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::warn!("Telegram agent processing failed: {}", e);
                        String::new()
                    }
                }
            })
        }).await;
        info!("Telegram relay: agent callback wired (unified inbox)");
    }

    if let Err(e) = relay.start().await {
        error!("Failed to start Telegram relay: {}", e);
    } else {
        info!("Telegram relay started (Bot API polling)");
    }
}

/// Start the Slack relay (Socket Mode WebSocket).
pub async fn start_slack_relay(config: &Config) {
    let Some(ref relay_config) = config.slack_relay else { return };

    let slack_relay_config = zeus_channels::SlackRelayConfig {
        bot_token: relay_config.bot_token.clone(),
        app_token: relay_config.app_token.clone(),
        channel_ids: relay_config
            .channel_ids
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        allowed_users: relay_config
            .allowed_users
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        require_mention_in_channels: relay_config.require_mention_in_channels,
        target_session: relay_config.target_session.clone(),
        max_queue: 100,
        rate_limit_per_minute: 30,
    };
    let slack_relay = zeus_channels::SlackRelay::new(slack_relay_config);

    if let Err(e) = slack_relay.start().await {
        error!("Failed to start Slack relay: {}", e);
    } else {
        info!("Slack relay started (Socket Mode)");
    }
}

/// Start the Matrix relay (matrix-sdk sync loop → agent inbox).
/// Requires the `matrix` feature. When compiled without it, logs a warning if configured.
#[cfg(feature = "matrix")]
pub async fn start_matrix_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.matrix_relay else { return };
    use zeus_channels::ChannelAdapter;

    let matrix_config = zeus_channels::MatrixConfig {
        homeserver: relay_config.homeserver.clone(),
        username: relay_config.username.clone(),
        password: relay_config.password.clone(),
        access_token: relay_config.access_token.clone(),
        user_id: relay_config.user_id.clone(),
        rooms: relay_config.rooms.clone(),
        display_name: relay_config.display_name.clone(),
        policy: relay_config.policy.clone(),
        account_id: None,
        allow_bots: None,
    };
    match zeus_channels::MatrixAdapter::new(matrix_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("Failed to start Matrix relay: {}", e);
            } else {
                info!("Matrix relay started (matrix-sdk sync loop)");
                let adapter_for_reply = adapter.clone();
                let prometheus_for_matrix = prometheus.clone();
                let inbox_for_matrix = agent_inbox.clone();
                tasks.push(tokio::spawn(async move {
                    info!("Matrix relay consumer started");
                    while let Some(msg) = rx.recv().await {
                        let preview = safe_preview(&msg.content, 50);
                        info!("Matrix relay: {}/{}: {}", msg.source.user_id,
                            msg.source.chat_id.as_deref().unwrap_or("dm"), preview);

                        let response = inbox_for_matrix.send_and_wait(
                            msg.content.clone(), None, vec![], 1800, false, None,
                        ).await.map_err(|e| zeus_core::Error::Internal(e));

                        match response {
                            Ok(reply) => {
                                info!("Matrix relay: agent replied ({} chars)", reply.len());
                                if let Err(e) = adapter_for_reply.send(&msg.source, &reply).await {
                                    error!("Matrix relay: failed to send reply: {}", e);
                                }
                                record_llm_call(&prometheus_for_matrix, true).await;
                            }
                            Err(e) => {
                                error!("Matrix relay: agent error: {}", e);
                                record_llm_call(&prometheus_for_matrix, false).await;
                            }
                        }
                    }
                    Ok(())
                }));
            }
        }
        Err(e) => error!("Failed to create Matrix relay adapter: {}", e),
    }
}

/// No-op Matrix relay when compiled without the `matrix` feature.
#[cfg(not(feature = "matrix"))]
pub async fn start_matrix_relay(
    config: &Config,
    _agent_inbox: &zeus_core::inbox::InboxSender,
    _prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    _tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    if config.matrix_relay.is_some() {
        tracing::warn!("Matrix relay configured but built without 'matrix' feature — skipping");
    }
}

/// Start the Signal relay (signal-cli JSON-RPC subprocess → agent inbox).
pub async fn start_signal_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.signal_relay else { return };
    use zeus_channels::ChannelAdapter;

    let signal_config = zeus_channels::SignalConfig {
        signal_cli_path: relay_config.signal_cli_path.clone(),
        phone: relay_config.phone.clone(),
        policy: relay_config.policy.clone(),
        account_id: None,
        allow_bots: None,
        http_host: relay_config.http_host.clone(),
        http_port: relay_config.http_port,
    };
    match zeus_channels::SignalAdapter::new(signal_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("Failed to start Signal relay: {}", e);
            } else {
                info!("Signal relay started (signal-cli JSON-RPC)");
                let adapter_for_reply = adapter.clone();
                let prometheus_for_signal = prometheus.clone();
                let inbox_for_signal = agent_inbox.clone();
                let allowed_senders = relay_config.allowed_senders.clone();
                tasks.push(tokio::spawn(async move {
                    info!("Signal relay consumer started");
                    if !allowed_senders.is_empty() {
                        info!("Signal relay: filtering to {} allowed sender(s)", allowed_senders.len());
                    }
                    while let Some(msg) = rx.recv().await {
                        // Filter by allowed senders if configured
                        if !allowed_senders.is_empty() {
                            let sender = &msg.source.user_id;
                            if !allowed_senders.iter().any(|s| s == sender) {
                                debug!("Signal relay: ignoring message from non-allowed sender: {}", sender);
                                continue;
                            }
                        }
                        let preview = safe_preview(&msg.content, 50);
                        info!("Signal relay: {}: {}", msg.source.user_id, preview);

                        let response = inbox_for_signal.send_and_wait(
                            msg.content.clone(), None, vec![], 1800, false, None,
                        ).await.map_err(|e| zeus_core::Error::Internal(e));

                        match response {
                            Ok(reply) => {
                                info!("Signal relay: agent replied ({} chars)", reply.len());
                                if let Err(e) = adapter_for_reply.send(&msg.source, &reply).await {
                                    error!("Signal relay: failed to send reply: {}", e);
                                }
                                record_llm_call(&prometheus_for_signal, true).await;
                            }
                            Err(e) => {
                                error!("Signal relay: agent error: {}", e);
                                record_llm_call(&prometheus_for_signal, false).await;
                            }
                        }
                    }
                    Ok(())
                }));
            }
        }
        Err(e) => error!("Failed to create Signal relay adapter: {}", e),
    }
}

/// Start the Email relay (IMAP polling → agent inbox).
pub async fn start_email_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.email_relay else { return };
    use zeus_channels::ChannelAdapter;

    let email_config = zeus_channels::EmailConfig {
        smtp_server: relay_config.smtp_server.clone(),
        smtp_port: relay_config.smtp_port,
        imap_server: relay_config.imap_server.clone(),
        imap_port: relay_config.imap_port,
        inbox_folder: relay_config.inbox_folder.clone(),
        email: relay_config.email.clone(),
        password: relay_config.password.clone(),
        use_tls: relay_config.use_tls,
        poll_interval_secs: relay_config.poll_interval_secs,
        policy: relay_config.policy.clone(),
    };
    match zeus_channels::EmailAdapter::new(email_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("Failed to start Email relay: {}", e);
            } else {
                info!("Email relay started (IMAP polling)");
                let adapter_for_reply = adapter.clone();
                let prometheus_for_email = prometheus.clone();
                let inbox_for_email = agent_inbox.clone();
                let allowed_senders = relay_config.allowed_senders.clone();
                tasks.push(tokio::spawn(async move {
                    info!("Email relay consumer started");
                    while let Some(msg) = rx.recv().await {
                        if !allowed_senders.is_empty() {
                            let sender = msg.source.user_id.to_lowercase();
                            if !allowed_senders.iter().any(|s| s.to_lowercase() == sender) {
                                debug!("Email relay: ignoring message from non-allowed sender: {}", sender);
                                continue;
                            }
                        }
                        let preview = safe_preview(&msg.content, 50);
                        info!("Email relay: {}: {}", msg.source.user_id, preview);

                        let response = inbox_for_email.send_and_wait(
                            msg.content.clone(), None, vec![], 1800, false, None,
                        ).await.map_err(|e| zeus_core::Error::Internal(e));

                        match response {
                            Ok(reply) => {
                                info!("Email relay: agent replied ({} chars)", reply.len());
                                if let Err(e) = adapter_for_reply.send(&msg.source, &reply).await {
                                    error!("Email relay: failed to send reply: {}", e);
                                }
                                record_llm_call(&prometheus_for_email, true).await;
                            }
                            Err(e) => {
                                error!("Email relay: agent error: {}", e);
                                record_llm_call(&prometheus_for_email, false).await;
                            }
                        }
                    }
                    Ok(())
                }));
            }
        }
        Err(e) => error!("Failed to create Email relay adapter: {}", e),
    }
}

/// Start the MQTT relay (rumqttc subscribe loop → agent inbox).
pub async fn start_mqtt_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.mqtt_relay else { return };
    use zeus_channels::ChannelAdapter;

    let mqtt_config = zeus_channels::MqttConfig {
        broker_url: relay_config.broker.clone(),
        port: relay_config.port,
        client_id: relay_config.client_id.clone(),
        topic_prefix: String::new(),
        qos: relay_config.qos,
        subscribe_topics: relay_config.topics.clone(),
        username: relay_config.username.clone(),
        password: relay_config.password.clone(),
        keep_alive_secs: relay_config.keep_alive_secs,
        clean_session: true,
        last_will_topic: None,
        last_will_message: None,
        channel_capacity: 256,
    };
    let adapter = Arc::new(zeus_channels::MqttAdapter::new(mqtt_config));
    let (tx, mut rx) = tokio::sync::mpsc::channel(500);

    if let Err(e) = adapter.start(tx).await {
        error!("Failed to start MQTT relay: {}", e);
    } else {
        info!("MQTT relay started (rumqttc subscribe loop)");
        let adapter_for_reply = adapter.clone();
        let prometheus_for_mqtt = prometheus.clone();
        let inbox_for_mqtt = agent_inbox.clone();
        let reply_prefix = relay_config.reply_topic_prefix.clone();
        tasks.push(tokio::spawn(async move {
            info!("MQTT relay consumer started");
            while let Some(msg) = rx.recv().await {
                let preview = safe_preview(&msg.content, 50);
                info!("MQTT relay: {}: {}",
                    msg.source.chat_id.as_deref().unwrap_or(&msg.source.user_id), preview);

                let response = inbox_for_mqtt.send_and_wait(
                    msg.content.clone(), None, vec![], 1800, false, None,
                ).await.map_err(|e| zeus_core::Error::Internal(e));

                match response {
                    Ok(reply) => {
                        info!("MQTT relay: agent replied ({} chars)", reply.len());
                        let reply_topic = if reply_prefix.is_empty() {
                            msg.source.chat_id.clone().unwrap_or_else(|| msg.source.user_id.clone())
                        } else {
                            let orig = msg.source.chat_id.as_deref().unwrap_or(&msg.source.user_id);
                            format!("{}{}", reply_prefix, orig)
                        };
                        let reply_target = zeus_channels::ChannelSource::with_chat("mqtt", &reply_topic, &reply_topic);
                        if let Err(e) = adapter_for_reply.send(&reply_target, &reply).await {
                            error!("MQTT relay: failed to publish reply: {}", e);
                        }
                        record_llm_call(&prometheus_for_mqtt, true).await;
                    }
                    Err(e) => {
                        error!("MQTT relay: agent error: {}", e);
                        record_llm_call(&prometheus_for_mqtt, false).await;
                    }
                }
            }
            Ok(())
        }));
    }
}

/// Start the WhatsApp relay (Baileys Bridge or Cloud API → agent inbox).
pub async fn start_whatsapp_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.whatsapp_relay else { return };
    use zeus_channels::ChannelAdapter;

    let mode = relay_config.mode.as_deref().unwrap_or("bridge");
    let wa_mode = if mode == "cloud_api" {
        zeus_channels::WhatsAppMode::CloudApi
    } else {
        zeus_channels::WhatsAppMode::Bridge
    };
    let wa_config = zeus_channels::WhatsAppConfig {
        mode: wa_mode.clone(),
        bridge_url: relay_config.bridge_url.clone(),
        access_token: relay_config.access_token.clone(),
        phone_number_id: relay_config.phone_number_id.clone(),
        verify_token: relay_config.verify_token.clone(),
        api_version: relay_config.api_version.clone().unwrap_or_else(|| "v21.0".to_string()),
        business_account_id: None,
        phone: relay_config.phone.clone(),
        policy: relay_config.policy.clone(),
        account_id: None,
        allow_bots: None,
    };
    match zeus_channels::WhatsAppAdapter::new_unchecked(wa_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("Failed to start WhatsApp relay: {}", e);
            } else {
                let mode_label = if wa_mode == zeus_channels::WhatsAppMode::CloudApi { "Cloud API" } else { "Baileys Bridge" };
                info!("WhatsApp relay started ({})", mode_label);
                let adapter_for_reply = adapter.clone();
                let prometheus_for_wa = prometheus.clone();
                let inbox_for_wa = agent_inbox.clone();
                let allowed_senders = relay_config.allowed_senders.clone();
                tasks.push(tokio::spawn(async move {
                    info!("WhatsApp relay consumer started");
                    if !allowed_senders.is_empty() {
                        info!("WhatsApp relay: filtering to {} allowed sender(s)", allowed_senders.len());
                    }
                    while let Some(msg) = rx.recv().await {
                        if !allowed_senders.is_empty() {
                            let sender = &msg.source.user_id;
                            if !allowed_senders.iter().any(|s| s == sender) {
                                debug!("WhatsApp relay: ignoring message from non-allowed sender: {}", sender);
                                continue;
                            }
                        }
                        let preview = safe_preview(&msg.content, 50);
                        info!("WhatsApp relay: {}/{}: {}", msg.source.user_id,
                            msg.source.chat_id.as_deref().unwrap_or("dm"), preview);

                        let response = inbox_for_wa.send_and_wait(
                            msg.content.clone(), None, vec![], 1800, false, None,
                        ).await.map_err(|e| zeus_core::Error::Internal(e));

                        match response {
                            Ok(reply) => {
                                info!("WhatsApp relay: agent replied ({} chars)", reply.len());
                                if let Err(e) = adapter_for_reply.send(&msg.source, &reply).await {
                                    error!("WhatsApp relay: failed to send reply: {}", e);
                                }
                                record_llm_call(&prometheus_for_wa, true).await;
                            }
                            Err(e) => {
                                error!("WhatsApp relay: agent error: {}", e);
                                record_llm_call(&prometheus_for_wa, false).await;
                            }
                        }
                    }
                    Ok(())
                }));
            }
        }
        Err(e) => error!("Failed to create WhatsApp relay adapter: {}", e),
    }
}

/// Start the Mattermost relay (WebSocket + REST → agent inbox).
pub async fn start_mattermost_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    let Some(ref relay_config) = config.mattermost_relay else { return };
    use zeus_channels::ChannelAdapter;

    let mm_config = zeus_channels::MattermostConfig {
        server_url: relay_config.server_url.clone(),
        token: relay_config.token.clone(),
        team_id: relay_config.team_id.clone(),
    };
    match zeus_channels::MattermostAdapter::new(mm_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("Failed to start Mattermost relay: {}", e);
            } else {
                info!("Mattermost relay started (WebSocket)");
                let adapter_for_reply = adapter.clone();
                let prometheus_for_mm = prometheus.clone();
                let inbox_for_mm = agent_inbox.clone();
                tasks.push(tokio::spawn(async move {
                    info!("Mattermost relay consumer started");
                    while let Some(msg) = rx.recv().await {
                        let preview = safe_preview(&msg.content, 50);
                        info!("Mattermost relay: {}/{}: {}", msg.source.user_id,
                            msg.source.chat_id.as_deref().unwrap_or("dm"), preview);

                        let response = inbox_for_mm.send_and_wait(
                            msg.content.clone(), None, vec![], 1800, false, None,
                        ).await.map_err(|e| zeus_core::Error::Internal(e));

                        match response {
                            Ok(reply) => {
                                info!("Mattermost relay: agent replied ({} chars)", reply.len());
                                if let Err(e) = adapter_for_reply.send(&msg.source, &reply).await {
                                    error!("Mattermost relay: failed to send reply: {}", e);
                                }
                                record_llm_call(&prometheus_for_mm, true).await;
                            }
                            Err(e) => {
                                error!("Mattermost relay: agent error: {}", e);
                                record_llm_call(&prometheus_for_mm, false).await;
                            }
                        }
                    }
                    Ok(())
                }));
            }
        }
        Err(e) => error!("Failed to create Mattermost relay adapter: {}", e),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Safely preview a string up to `max` chars, respecting char boundaries.
fn safe_preview(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) && end < s.len() {
            end += 1;
        }
        &s[..end]
    }
}

/// Start the X (Twitter) relay (mentions polling → agent inbox → reply).
///
/// Gracefully no-ops when `[channels.x_twitter]` is absent or credentials are empty,
/// matching the pattern of every other relay. The underlying `XAdapter` polls mentions
/// on its own interval (default 60s) and feeds them through the shared agent inbox.
pub async fn start_x_relay(
    config: &Config,
    agent_inbox: &zeus_core::inbox::InboxSender,
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    tasks: &mut Vec<JoinHandle<Result<()>>>,
) {
    use zeus_channels::ChannelAdapter;

    let Some(ref channels) = config.channels else {
        debug!("X relay: [channels] block absent — skipping");
        return;
    };
    let Some(ref xt) = channels.x_twitter else {
        debug!("X relay: [channels.x_twitter] absent — skipping");
        return;
    };

    // Minimum viable credentials: bearer token OR full OAuth 1.0a set.
    let has_bearer = !xt.bearer_token.is_empty();
    let has_oauth1 = !xt.api_key.is_empty()
        && !xt.api_secret.is_empty()
        && !xt.access_token.is_empty()
        && !xt.access_token_secret.is_empty();
    if !has_bearer && !has_oauth1 {
        info!("X relay: not configured (no credentials) — skipping");
        return;
    }

    let x_config = zeus_channels::XConfig {
        bearer_token: xt.bearer_token.clone(),
        api_key: xt.api_key.clone(),
        api_secret: xt.api_secret.clone(),
        access_token: xt.access_token.clone(),
        access_token_secret: xt.access_token_secret.clone(),
        client_id: xt.client_id.clone(),
        client_secret: xt.client_secret.clone(),
        oauth2_access_token: String::new(),
        oauth2_refresh_token: String::new(),
        oauth2_expires_at: 0,
        user_id: None,
        poll_interval_secs: xt.poll_interval_secs,
        auto_reply: xt.auto_reply,
    };

    match zeus_channels::XAdapter::new(x_config).await {
        Ok(adapter) => {
            let adapter = Arc::new(adapter);
            let (tx, mut rx) = tokio::sync::mpsc::channel(500);

            if let Err(e) = adapter.start(tx).await {
                error!("X relay: failed to start adapter: {}", e);
                return;
            }

            info!(
                "X relay started (polling every {}s, auto_reply={})",
                xt.poll_interval_secs.unwrap_or(60),
                xt.auto_reply
            );

            let adapter_for_reply = adapter.clone();
            let prometheus_for_x = prometheus.clone();
            let inbox_for_x = agent_inbox.clone();
            let auto_reply = xt.auto_reply;

            tasks.push(tokio::spawn(async move {
                info!("X relay consumer started");
                while let Some(msg) = rx.recv().await {
                    let preview = safe_preview(&msg.content, 80);
                    info!(
                        "X relay: mention from @{}: {}",
                        msg.source.user_id, preview
                    );

                    let response = inbox_for_x
                        .send_and_wait(msg.content.clone(), None, vec![], 1800, false, None)
                        .await
                        .map_err(zeus_core::Error::Internal);

                    match response {
                        Ok(reply) => {
                            info!("X relay: agent replied ({} chars)", reply.len());
                            if auto_reply {
                                if let Err(e) =
                                    adapter_for_reply.send(&msg.source, &reply).await
                                {
                                    error!("X relay: failed to post reply tweet: {}", e);
                                }
                            } else {
                                debug!("X relay: auto_reply=false — not posting tweet");
                            }
                            record_llm_call(&prometheus_for_x, true).await;
                        }
                        Err(e) => {
                            error!("X relay: agent error: {}", e);
                            record_llm_call(&prometheus_for_x, false).await;
                        }
                    }
                }
                Ok(())
            }));
        }
        Err(e) => error!("X relay: failed to create adapter: {}", e),
    }
}

/// Record an LLM call in Prometheus monitoring (if available).
async fn record_llm_call(
    prometheus: &Option<Arc<RwLock<zeus_prometheus::Prometheus>>>,
    success: bool,
) {
    if let Some(prom) = prometheus {
        let prom_guard = prom.read().await;
        prom_guard.monitor().record_llm_call(0, success);
    }
}
