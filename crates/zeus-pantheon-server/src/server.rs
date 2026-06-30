//! WebSocket accept loop — tokio-tungstenite, with optional TLS (tokio-rustls).
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    auth::{resolve_tier, verify_token},
    config::PantheonServerConfig,
    protocol::{ClientMessage, ErrorCode, ServerMessage},
    rate_limiter::RateLimiter,
    state::ServerState,
};

pub async fn run(config: PantheonServerConfig, state: Arc<ServerState>) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let listener = TcpListener::bind(&addr).await?;
    info!("Pantheon server listening on ws://{}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                info!("New connection from {}", peer);
                let state = Arc::clone(&state);
                let config = config.clone();
                tokio::spawn(handle_connection(stream, peer, state, config));
            }
            Err(e) => error!("Accept error: {}", e),
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    state: Arc<ServerState>,
    config: PantheonServerConfig,
) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => { warn!("WS handshake failed from {}: {}", peer, e); return; }
    };

    let (mut sink, mut source) = ws.split();
    let conn_id = Uuid::new_v4().to_string();
    let mut authed_user: Option<crate::protocol::UserInfo> = None;
    let mut rate_limiter = RateLimiter::with_rate(config.rate_burst, config.rate_per_sec);

    // Helper: send a ServerMessage as JSON text frame
    macro_rules! send {
        ($msg:expr) => {{
            if let Ok(json) = serde_json::to_string(&$msg) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }};
    }

    while let Some(raw) = source.next().await {
        let raw = match raw {
            Ok(r) => r,
            Err(e) => { warn!("WS read error from {}: {}", peer, e); break; }
        };

        let text = match raw {
            Message::Text(t) => t,
            Message::Close(_) => break,
            Message::Ping(d) => { let _ = sink.send(Message::Pong(d)).await; continue; }
            _ => continue,
        };

        let client_msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(_) => {
                send!(ServerMessage::err(ErrorCode::InvalidMessage, "Could not parse message"));
                continue;
            }
        };

        match client_msg {
            ClientMessage::Auth { user_id, display_name, token, nonce, agent } => {
                if verify_token(&config.channel_key, &user_id, &nonce, &token) {
                    // Phase 5: nick reservation
                    if config.nick_reservation {
                        if let Err(reason) = state.reserve_nick(&display_name, &conn_id) {
                            send!(ServerMessage::AuthErr { reason });
                            continue;
                        }
                    }
                    let tier = resolve_tier(&user_id, &config.admin_ids);
                    let user = crate::protocol::UserInfo {
                        user_id: user_id.clone(),
                        display_name,
                        tier: tier.clone(),
                        agent,
                    };
                    state.register_connection(&conn_id, user.clone()).await;
                    authed_user = Some(user);

                    // Auto-join default channels
                    let mut joined = Vec::new();
                    for ch in &config.default_channels {
                        state.join_channel(&conn_id, ch).await;
                        joined.push(ch.clone());
                    }
                    send!(ServerMessage::AuthOk { user_id, tier, channels: joined });
                    // Phase 5: MOTD
                    if !config.motd.is_empty() {
                        send!(ServerMessage::Motd { text: config.motd.clone() });
                    }
                } else {
                    send!(ServerMessage::AuthErr { reason: "Invalid token".into() });
                }
            }

            _ if authed_user.is_none() => {
                send!(ServerMessage::err(ErrorCode::Unauthorized, "Authenticate first"));
            }

            ClientMessage::Join { channel } => {
                state.join_channel(&conn_id, &channel).await;
                let members = state.channel_members(&channel).await;
                let topic = state.channel_topic(&channel).await;
                send!(ServerMessage::Joined { channel, members, topic });
            }

            ClientMessage::Part { channel } => {
                state.part_channel(&conn_id, &channel).await;
                send!(ServerMessage::Parted { channel });
            }

            ClientMessage::Msg { channel, content, message_type } => {
                if let Some(user) = &authed_user {
                    // Phase 5: rate limiting
                    if !rate_limiter.check() {
                        let retry = rate_limiter.retry_after_secs();
                        send!(ServerMessage::err(
                            ErrorCode::RateLimited,
                            format!("Rate limited — retry in {:.1}s", retry)
                        ));
                        continue;
                    }
                    state.broadcast_msg(&channel, user.clone(), content, message_type).await;
                }
            }

            ClientMessage::Topic { channel, topic } => {
                if let Some(user) = &authed_user {
                    state.set_topic(&channel, &topic, &user.user_id).await;
                    // broadcast handled inside set_topic
                }
            }

            ClientMessage::Who { channel } => {
                let members = state.channel_members(&channel).await;
                send!(ServerMessage::WhoReply { channel, members });
            }

            ClientMessage::Ping { nonce } => {
                send!(ServerMessage::Pong { nonce });
            }
        }
    }

    // Cleanup on disconnect
    state.remove_connection(&conn_id).await;
    info!("Connection {} ({}) disconnected", conn_id, peer);
}
