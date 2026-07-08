//! Channel manager construction extracted from `Agent::with_subsystems`.
//!
//! This module owns the 503-LOC inline construction block that wires every
//! platform-channel adapter (Telegram, Discord+voice, Slack, X, Matrix-relay,
//! IRC, Mattermost, Signal, WhatsApp, Email, iMessage, MQTT) into a single
//! `ChannelManager`. Extracted so the standalone MCP binary (`zeus-mcp`)
//! under `--full` can wire a real `ChannelManager` instead of `None`.
//!
//! See `crates/zeus-mcp/src/bin/server.rs` for the deferred wire-in site.

use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use zeus_core::{Config, Result};
use zeus_channels::{ChannelManager, ChannelMessage};

/// Build a `ChannelManager` (and its inbound receiver) from a `Config`.
///
/// Mirrors the inline construction previously living at
/// `agent_loop.rs:603-1114`. Behavior is identical: env-merge with
/// `config.channels`, then per-adapter setup with all platform-specific
/// telegram_bot / songbird / matrix-relay / IRC auto-join wiring preserved.
///
/// Returns `(None, None)` when no channels are configured (neither
/// `config.channels` nor env vars provide a `ChannelsConfig`), or when
/// channels are disabled via `[gateway] enable_channels = false` /
/// `--no-channels`.
///
/// The disabled check runs *before* any env read: a disabled instance must
/// never create adapters from an inherited `DISCORD_BOT_TOKEN` (etc.) —
/// under multi-instance that meant N instances = N duplicate Discord
/// sessions off one token. The "Channels: disabled" startup log must be true.
pub async fn build_channel_manager_from_config(
    config: &Config,
) -> Result<(Option<Arc<ChannelManager>>, Option<mpsc::Receiver<ChannelMessage>>)> {
    if !config
        .gateway
        .as_ref()
        .map(|g| g.enable_channels)
        .unwrap_or(true)
    {
        info!("Channels disabled ([gateway].enable_channels=false) — skipping adapter creation and env-merge");
        return Ok((None, None));
    }
    // Initialize channel manager if configured (or auto-detect from env vars)
    let env_channels = zeus_core::ChannelsConfig::from_env();
    let effective_channels: Option<zeus_core::ChannelsConfig> = match &config.channels {
        Some(cc) => {
            let mut merged = cc.clone();
            merged.merge_env();
            Some(merged)
        }
        None => env_channels,
    };
    let (channels, channel_rx) = if let Some(ref cc) = effective_channels {
        let mut manager = ChannelManager::new(1000);

        if let Some(ref tc) = cc.telegram {
            let tg_config = zeus_channels::TelegramConfig {
                api_id: tc.api_id,
                api_hash: tc.api_hash.clone(),
                bot_token: tc.bot_token.clone(),
                phone: if tc.phone.is_empty() {
                    None
                } else {
                    Some(tc.phone.clone())
                },
                session_path: tc.session_file.clone(),
                allow_bots: None,
            };
            match zeus_channels::TelegramAdapter::new(tg_config).await {
                Ok(adapter) => {
                    info!("Telegram channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Telegram adapter: {}", e),
            }
        } else if let Some(ref relay) = config.telegram_relay {
            // Auto-register a Bot API outbound adapter when [channels.telegram] is absent
            // but [telegram_relay] has a token. Fixes nodes onboarded with Bot API relay
            // that never had a [channels.telegram] section written.
            if !relay.bot_token.is_empty() {
                let bot_cfg = zeus_channels::telegram_bot::TelegramBotConfig::new(&relay.bot_token);
                let adapter = zeus_channels::telegram_bot::TelegramBotAdapter::new(bot_cfg);
                info!("Telegram Bot API outbound adapter auto-registered from [telegram_relay]");
                manager.add_adapter(Box::new(adapter));
            }
        }

        // Auto-register TelegramBotAdapter from [telegram_relay] if [channels.telegram] is absent.
        // This gives outbound capability to the `message` tool when only the relay is configured.
        if cc.telegram.is_none()
            && let Some(ref relay) = config.telegram_relay
            && !relay.bot_token.is_empty()
        {
            let bot_config = zeus_channels::telegram_bot::TelegramBotConfig {
                bot_token: relay.bot_token.clone(),
                default_chat_id: relay.chat_id.parse::<i64>().ok(),
                poll_timeout_secs: None,
            };
            let adapter = zeus_channels::telegram_bot::TelegramBotAdapter::new(bot_config);
            info!("Telegram Bot adapter auto-registered from [telegram_relay]");
            manager.add_adapter(Box::new(adapter));
        }

        if let Some(ref dc) = cc.discord {
            // config.toml is sole source of truth — no env var fallback
            let token = dc.token.clone();
            // Only create the default (top-level) adapter if a token is available
            // AND no named accounts are configured. When accounts exist, the top-level
            // section is just a container — creating an adapter here would duplicate
            // connections if the env var or config has the same token as an account.
            if !token.is_empty() && dc.accounts.is_empty() {
            let discord_config = zeus_channels::DiscordConfig {
                bot_token: token,
                application_id: dc.application_id,
                allowed_guilds: vec![],
                policy: dc.policy.clone(),
                allow_bots: dc.allow_bots.clone(),
                ..Default::default()
            };
            match zeus_channels::DiscordAdapter::new(discord_config).await {
                Ok(adapter) => {
                    // Wire voice support if configured
                    if let Some(ref vc) = dc.voice
                        && vc.enabled
                    {
                        let voice_config = zeus_channels::DiscordVoiceConfig {
                            auto_join_channels: vc.auto_join_channels.clone(),
                            min_speech_ms: vc.min_speech_ms,
                            silence_timeout_ms: vc.silence_timeout_ms,
                            energy_threshold: vc.energy_threshold,
                            tts_voice: vc
                                .tts_voice
                                .clone()
                                .unwrap_or_else(|| "en_US-amy-medium".to_string()),
                            tts_provider: vc
                                .tts_provider
                                .clone()
                                .unwrap_or_else(|| "piper".to_string()),
                            piper_url: vc.piper_url.clone(),
                            stt_provider: vc.stt_provider.clone(),
                            require_wake_word: vc.require_wake_word,
                            wake_words: if vc.wake_words.is_empty() {
                                vec!["zeus".to_string(), "hey zeus".to_string()]
                            } else {
                                vc.wake_words.clone()
                            },
                        };
                        let (mut voice_session, songbird) =
                            zeus_channels::DiscordVoiceSession::new(voice_config.clone());
                        adapter.set_songbird(songbird).await;

                        // Auto-join configured voice channels and bridge
                        // transcripts into the channel message pipeline
                        if !voice_config.auto_join_channels.is_empty() {
                            let inbound_tx = manager.inbound_tx();
                            for entry in &voice_config.auto_join_channels {
                                // Format: "guild_id:channel_id"
                                let parts: Vec<&str> = entry.split(':').collect();
                                if parts.len() != 2 {
                                    warn!("Invalid auto_join_channels entry '{}', expected 'guild_id:channel_id'", entry);
                                    continue;
                                }
                                let guild_id: u64 = match parts[0].parse() {
                                    Ok(id) => id,
                                    Err(_) => {
                                        warn!("Invalid guild_id in auto_join_channels: '{}'", parts[0]);
                                        continue;
                                    }
                                };
                                let channel_id: u64 = match parts[1].parse() {
                                    Ok(id) => id,
                                    Err(_) => {
                                        warn!("Invalid channel_id in auto_join_channels: '{}'", parts[1]);
                                        continue;
                                    }
                                };
                                match voice_session.join(guild_id, channel_id).await {
                                    Ok(mut transcript_rx) => {
                                        let tx = inbound_tx.clone();
                                        let chan_id_str = channel_id.to_string();
                                        tokio::spawn(async move {
                                            while let Some(transcript) = transcript_rx.recv().await {
                                                let source = zeus_channels::ChannelSource::with_chat(
                                                    "discord",
                                                    &transcript.user_id.to_string(),
                                                    &chan_id_str,
                                                );
                                                let content = format!(
                                                    "[Voice transcription]: {}",
                                                    transcript.text
                                                );
                                                let msg = zeus_channels::ChannelMessage::new(source, content);
                                                if tx.send(msg).await.is_err() {
                                                    debug!("Voice transcript bridge: inbound_tx closed");
                                                    break;
                                                }
                                            }
                                            debug!("Voice transcript bridge exited for channel {}", chan_id_str);
                                        });
                                        info!(guild_id, channel_id, "Auto-joined voice channel, transcript bridge active");
                                    }
                                    Err(e) => {
                                        warn!(guild_id, channel_id, error = %e, "Failed to auto-join voice channel");
                                    }
                                }
                            }
                        }
                        info!("Discord voice support enabled");
                    }
                    info!("Discord channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Discord adapter: {}", e),
            }
            }

            // S35: Spawn additional Discord adapters for named accounts
            for (acct_key, acct_cfg) in &dc.accounts {
                // config.toml is sole source of truth — no env var override
                let acct_token = acct_cfg.token.clone();
                if acct_token.is_empty() {
                    warn!(
                        "Discord account '{}': no token in config.toml",
                        acct_key
                    );
                    continue;
                }
                let acct_discord_config = zeus_channels::DiscordConfig {
                    bot_token: acct_token,
                    application_id: acct_cfg.application_id,
                    allowed_guilds: vec![],
                    policy: acct_cfg.policy.clone(),
                    webhook_url: acct_cfg.webhook_url.clone(),
                    account_id: Some(acct_key.clone()),
                    allow_bots: acct_cfg.allow_bots.clone().or_else(|| dc.allow_bots.clone()),
                    ..Default::default()
                };
                match zeus_channels::DiscordAdapter::new(acct_discord_config).await {
                    Ok(adapter) => {
                        info!(
                            "Discord account '{}' adapter created (agent: {:?})",
                            acct_key, acct_cfg.agent_id
                        );
                        manager.add_adapter(Box::new(adapter));
                    }
                    Err(e) => warn!(
                        "Failed to create Discord adapter for account '{}': {}",
                        acct_key, e
                    ),
                }
            }
        }

        if let Some(ref sc) = cc.slack {
            let slack_config = zeus_channels::SlackConfig {
                bot_token: sc.bot_token.clone(),
                app_token: Some(sc.app_token.clone()),
                signing_secret: None,
                policy: sc.policy.clone(),
                ..Default::default()
            };
            match zeus_channels::SlackAdapter::new(slack_config).await {
                Ok(adapter) => {
                    info!("Slack channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Slack adapter: {}", e),
            }
        }

        if let Some(ref ec) = cc.email {
            let email_config = zeus_channels::EmailConfig {
                smtp_server: ec.smtp_host.clone(),
                smtp_port: ec.smtp_port,
                imap_server: ec.imap_host.clone(),
                imap_port: ec.imap_port,
                inbox_folder: "INBOX".to_string(),
                email: ec.username.clone(),
                password: ec.password.clone(),
                use_tls: ec.use_tls,
                poll_interval_secs: 60,
                policy: ec.policy.clone(),
            };
            match zeus_channels::EmailAdapter::new(email_config).await {
                Ok(adapter) => {
                    info!("Email channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Email adapter: {}", e),
            }
        }

        if let Some(ref mc) = cc.mqtt {
            let mqtt_config = zeus_channels::MqttConfig {
                broker_url: mc.broker_url.clone(),
                port: mc.port,
                client_id: mc.client_id.clone().unwrap_or_else(|| {
                    format!(
                        "zeus-{}",
                        uuid::Uuid::new_v4()
                            .to_string()
                            .split('-')
                            .next()
                            .unwrap_or("agent")
                    )
                }),
                topic_prefix: mc.topic_prefix.clone(),
                qos: mc.qos,
                subscribe_topics: mc.subscribe_topics.clone(),
                username: mc.username.clone(),
                password: mc.password.clone(),
                ..Default::default()
            };
            let adapter = zeus_channels::MqttAdapter::new(mqtt_config);
            info!("MQTT channel adapter created");
            manager.add_adapter(Box::new(adapter));
        }

        if let Some(ref wc) = cc.whatsapp {
            let wa_config = zeus_channels::WhatsAppConfig {
                bridge_url: wc.bridge_url.clone(),
                policy: wc.policy.clone(),
                ..Default::default()
            };
            match zeus_channels::WhatsAppAdapter::new(wa_config).await {
                Ok(adapter) => {
                    info!("WhatsApp channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create WhatsApp adapter: {}", e),
            }

            // S43: Spawn additional WhatsApp adapters for named accounts
            for (acct_key, acct_cfg) in &wc.accounts {
                let acct_wa_config = zeus_channels::WhatsAppConfig {
                    bridge_url: acct_cfg.bridge_url.clone().unwrap_or_else(|| wc.bridge_url.clone()),
                    phone: acct_cfg.phone.clone(),
                    access_token: acct_cfg.access_token.clone(),
                    phone_number_id: acct_cfg.phone_number_id.clone(),
                    mode: match acct_cfg.mode.as_deref() {
                        Some("cloud_api") => zeus_channels::WhatsAppMode::CloudApi,
                        _ => zeus_channels::WhatsAppMode::Bridge,
                    },
                    policy: acct_cfg.policy.clone().or_else(|| wc.policy.clone()),
                    account_id: Some(acct_key.clone()),
                    allow_bots: acct_cfg.allow_bots.clone().or_else(|| wc.allow_bots.clone()),
                    ..Default::default()
                };
                match zeus_channels::WhatsAppAdapter::new(acct_wa_config).await {
                    Ok(adapter) => {
                        info!("WhatsApp account '{}' adapter created (agent: {:?})", acct_key, acct_cfg.agent_id);
                        manager.add_adapter(Box::new(adapter));
                    }
                    Err(e) => warn!("Failed to create WhatsApp adapter for account '{}': {}", acct_key, e),
                }
            }
        }

        if let Some(ref sc) = cc.signal {
            let signal_config = zeus_channels::SignalConfig {
                signal_cli_path: sc.signal_cli_path.clone(),
                phone: sc.phone.clone(),
                policy: sc.policy.clone(),
                ..Default::default()
            };
            match zeus_channels::SignalAdapter::new(signal_config).await {
                Ok(adapter) => {
                    info!("Signal channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Signal adapter: {}", e),
            }

            // S43: Spawn additional Signal adapters for named accounts
            for (acct_key, acct_cfg) in &sc.accounts {
                let acct_signal_config = zeus_channels::SignalConfig {
                    signal_cli_path: acct_cfg.signal_cli_path.clone()
                        .unwrap_or_else(|| sc.signal_cli_path.clone()),
                    phone: acct_cfg.phone.clone(),
                    policy: acct_cfg.policy.clone().or_else(|| sc.policy.clone()),
                    account_id: Some(acct_key.clone()),
                    allow_bots: acct_cfg.allow_bots.clone().or_else(|| sc.allow_bots.clone()),
                    http_host: "127.0.0.1".to_string(),
                    http_port: 8080,
                };
                match zeus_channels::SignalAdapter::new(acct_signal_config).await {
                    Ok(adapter) => {
                        info!("Signal account '{}' adapter created (agent: {:?})", acct_key, acct_cfg.agent_id);
                        manager.add_adapter(Box::new(adapter));
                    }
                    Err(e) => warn!("Failed to create Signal adapter for account '{}': {}", acct_key, e),
                }
            }
        }

        #[cfg(feature = "matrix")]
        if let Some(ref mx) = cc.matrix {
            let access_token = if mx.access_token.is_empty() {
                None
            } else {
                Some(mx.access_token.clone())
            };
            let matrix_config = zeus_channels::MatrixConfig {
                homeserver: mx.homeserver.clone(),
                username: mx.username.clone(),
                password: mx.password.clone(),
                access_token,
                user_id: mx.user_id.clone(),
                rooms: mx.rooms.clone(),
                display_name: mx.display_name.clone(),
                policy: mx.policy.clone(),
                ..Default::default()
            };
            match zeus_channels::MatrixAdapter::new(matrix_config).await {
                Ok(adapter) => {
                    info!("Matrix channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Matrix adapter: {}", e),
            }

            // S43: Spawn additional Matrix adapters for named accounts
            for (acct_key, acct_cfg) in &mx.accounts {
                let acct_access_token = acct_cfg.access_token.clone()
                    .filter(|t| !t.is_empty());
                let acct_matrix_config = zeus_channels::MatrixConfig {
                    homeserver: acct_cfg.homeserver.clone()
                        .unwrap_or_else(|| mx.homeserver.clone()),
                    username: acct_cfg.username.clone().or_else(|| mx.username.clone()),
                    password: acct_cfg.password.clone(),
                    access_token: acct_access_token,
                    user_id: acct_cfg.user_id.clone(),
                    rooms: if acct_cfg.rooms.is_empty() { mx.rooms.clone() } else { acct_cfg.rooms.clone() },
                    display_name: acct_cfg.display_name.clone(),
                    policy: acct_cfg.policy.clone().or_else(|| mx.policy.clone()),
                    account_id: Some(acct_key.clone()),
                    allow_bots: acct_cfg.allow_bots.clone().or_else(|| mx.allow_bots.clone()),
                };
                match zeus_channels::MatrixAdapter::new(acct_matrix_config).await {
                    Ok(adapter) => {
                        info!("Matrix account '{}' adapter created (agent: {:?})", acct_key, acct_cfg.agent_id);
                        manager.add_adapter(Box::new(adapter));
                    }
                    Err(e) => warn!("Failed to create Matrix adapter for account '{}': {}", acct_key, e),
                }
            }
        }

        // IRC adapter
        if let Some(ref ic) = cc.irc {
            let irc_config = zeus_channels::IrcConfig {
                server: ic.server.clone(),
                port: ic.port,
                nick: ic.nick.clone(),
                username: ic.username.clone(),
                channels: ic.channels.clone(),
                use_tls: ic.use_tls,
                nickserv_password: ic.nickserv_password.clone(),
                ..Default::default()
            };
            let adapter = zeus_channels::IrcAdapter::new(irc_config);
            info!("IRC channel adapter created");
            manager.add_adapter(Box::new(adapter));
        }

        // MQTT adapter
        if let Some(ref mq) = cc.mqtt {
            let mqtt_config = zeus_channels::MqttConfig {
                broker_url: mq.broker_url.clone(),
                port: mq.port,
                topic_prefix: mq.topic_prefix.clone(),
                client_id: mq.client_id.clone().unwrap_or_default(),
                qos: mq.qos,
                subscribe_topics: mq.subscribe_topics.clone(),
                username: mq.username.clone(),
                password: mq.password.clone(),
                ..Default::default()
            };
            let adapter = zeus_channels::MqttAdapter::new(mqtt_config);
            info!("MQTT channel adapter created");
            manager.add_adapter(Box::new(adapter));
        }

        // Mattermost adapter
        if let Some(ref mm) = cc.mattermost {
            let mm_config = zeus_channels::MattermostConfig {
                server_url: mm.server_url.clone(),
                token: mm.token.clone(),
                team_id: mm.team_id.clone(),
            };
            match zeus_channels::MattermostAdapter::new(mm_config).await {
                Ok(adapter) => {
                    info!("Mattermost channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create Mattermost adapter: {}", e),
            }
        }

        // X (Twitter) adapter
        if let Some(ref xt) = cc.x_twitter {
            let x_config = zeus_channels::XConfig {
                bearer_token: xt.bearer_token.clone(),
                api_key: xt.consumer_key.clone(),
                api_secret: xt.consumer_key_secret.clone(),
                access_token: xt.access_token.clone(),
                access_token_secret: xt.access_token_secret.clone(),
                // OAuth 2.0 app credentials — required for the PKCE user-context
                // flow (build_authorize_url / token exchange / silent refresh).
                // Previously dropped here, which stranded [channels.x_twitter]
                // client_id/client_secret configs: the adapter fell back to
                // OAuth 1.0a or bearer-only even when OAuth2 was configured.
                client_id: xt.client_id.clone(),
                client_secret: xt.client_secret.clone(),
                poll_interval_secs: xt.poll_interval_secs,
                auto_reply: xt.auto_reply,
                ..Default::default()
            };
            match zeus_channels::XAdapter::new(x_config).await {
                Ok(adapter) => {
                    info!("X (Twitter) channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create X (Twitter) adapter: {}", e),
            }
        }

        // S82: Wire iMessage adapter if configured
        // iMessage: macOS-only, no credentials needed (uses AppleScript)
        #[cfg(target_os = "macos")]
        {
            let imessage_config = zeus_channels::IMessageConfig::default();
            match zeus_channels::IMessageAdapter::new(imessage_config).await {
                Ok(adapter) => {
                    info!("iMessage channel adapter created");
                    manager.add_adapter(Box::new(adapter));
                }
                Err(e) => warn!("Failed to create iMessage adapter: {}", e),
            }
        }

        // Start all channel adapters in background so slow connections
        // (e.g. IRC ~75s) don't block API boot.
        let manager_arc = Arc::new(manager);
        let start_manager = Arc::clone(&manager_arc);
        tokio::spawn(async move {
            if let Err(e) = start_manager.start_all().await {
                warn!("Failed to start channel adapters: {}", e);
            }
            let connected = start_manager.connected_channels();
            if !connected.is_empty() {
                info!("Channel adapters connected: {:?}", connected);
            }
        });

        let rx = manager_arc.take_receiver();
        (Some(manager_arc), rx)
    } else {
        (None, None)
    };

    Ok((channels, channel_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `enable_channels = false` must short-circuit BEFORE any config/env
    /// adapter construction: no manager, no receiver — even when a Discord
    /// channel is fully configured. This is the `--no-channels` adapter-leak
    /// regression guard (N instances = N duplicate Discord sessions off one
    /// inherited DISCORD_BOT_TOKEN).
    #[tokio::test]
    async fn disabled_channels_skips_adapter_creation_even_with_config() {
        let mut config = Config::default();
        config.gateway = Some(zeus_core::GatewayConfig {
            enable_channels: false,
            ..Default::default()
        });
        // DiscordChannelConfig has no Default impl — deserialize a minimal
        // config the way config.toml would provide it.
        let discord: zeus_core::DiscordChannelConfig =
            toml::from_str(r#"token = "fake-token-should-never-be-used""#)
                .expect("minimal discord config must deserialize");
        config.channels = Some(zeus_core::ChannelsConfig {
            discord: Some(discord),
            ..Default::default()
        });

        let (channels, rx) = build_channel_manager_from_config(&config)
            .await
            .expect("builder must not error on disabled path");
        assert!(channels.is_none(), "adapters must not be created when channels are disabled");
        assert!(rx.is_none(), "no receiver when channels are disabled");
    }

    /// Default (no [gateway] section) keeps the historical behavior:
    /// channels enabled, builder proceeds to config/env resolution.
    #[tokio::test]
    async fn no_gateway_section_defaults_to_enabled_path() {
        let mut config = Config::default();
        config.gateway = None;
        config.channels = None; // and no env in test context ⇒ (None, None) via normal path
        // Guard: if a dev machine exports DISCORD_BOT_TOKEN etc., from_env()
        // would kick in — serialize and scrub the vars this test cares about.
        let vars = ["DISCORD_BOT_TOKEN", "SLACK_BOT_TOKEN", "SLACK_APP_TOKEN",
                    "MATRIX_HOMESERVER", "MATRIX_ACCESS_TOKEN", "SIGNAL_PHONE_NUMBER"];
        let saved: Vec<_> = vars.iter().map(|v| (v, std::env::var(v).ok())).collect();
        // SAFETY: test-only, single-threaded access to these vars within this test.
        for v in vars {
            unsafe { std::env::remove_var(v) };
        }

        let result = build_channel_manager_from_config(&config).await;

        for (v, val) in saved {
            if let Some(val) = val {
                // SAFETY: restoring prior test-scoped state.
                unsafe { std::env::set_var(v, val) };
            }
        }
        let (channels, rx) = result.expect("builder must not error");
        assert!(channels.is_none());
        assert!(rx.is_none());
    }
}
