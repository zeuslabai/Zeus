//! Shared server state — connections, channels, presence.
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::broadcast;
use uuid::Uuid;
use chrono::Utc;

use crate::protocol::{MessageKind, ServerMessage, UserInfo};

const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug)]
struct ChannelState {
    topic: String,
    /// conn_ids currently in this channel
    members: Vec<String>,
    tx: broadcast::Sender<ServerMessage>,
}

#[derive(Debug)]
struct Connection {
    user: UserInfo,
    channels: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ServerState {
    connections: DashMap<String, Connection>,
    channels: DashMap<String, ChannelState>,
    /// nick → conn_id reservation map (Phase 5)
    nicks: DashMap<String, String>,
}

impl ServerState {
    pub fn new(default_channels: &[String], _history_limit: usize) -> Arc<Self> {
        let state = Arc::new(Self::default());
        for ch in default_channels {
            let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
            state.channels.insert(ch.clone(), ChannelState {
                topic: String::new(),
                members: Vec::new(),
                tx,
            });
        }
        state
    }

    /// Try to reserve a nick. Returns Ok(()) if available, Err if taken by another conn.
    pub fn reserve_nick(&self, nick: &str, conn_id: &str) -> Result<(), String> {
        use dashmap::mapref::entry::Entry;
        match self.nicks.entry(nick.to_string()) {
            Entry::Vacant(e) => { e.insert(conn_id.to_string()); Ok(()) }
            Entry::Occupied(e) => {
                if e.get() == conn_id {
                    Ok(()) // same conn re-authing
                } else {
                    Err(format!("Nick '{}' is already in use", nick))
                }
            }
        }
    }

    pub async fn register_connection(&self, conn_id: &str, user: UserInfo) {
        self.connections.insert(conn_id.to_string(), Connection {
            user,
            channels: Vec::new(),
        });
    }

    pub async fn remove_connection(&self, conn_id: &str) {
        if let Some((_, conn)) = self.connections.remove(conn_id) {
            // Release nick reservation
            self.nicks.retain(|_, v| v != conn_id);
            for ch in &conn.channels {
                if let Some(mut ch_state) = self.channels.get_mut(ch) {
                    ch_state.members.retain(|id| id != conn_id);
                    let _ = ch_state.tx.send(ServerMessage::PresencePart {
                        channel: ch.clone(),
                        user_id: conn.user.user_id.clone(),
                    });
                }
            }
        }
    }

    pub async fn join_channel(&self, conn_id: &str, channel: &str) {
        // Ensure channel exists
        if !self.channels.contains_key(channel) {
            let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
            self.channels.insert(channel.to_string(), ChannelState {
                topic: String::new(),
                members: Vec::new(),
                tx,
            });
        }

        let user = self.connections.get(conn_id).map(|c| c.user.clone());
        if let Some(user) = user {
            if let Some(mut ch) = self.channels.get_mut(channel) {
                if !ch.members.contains(&conn_id.to_string()) {
                    ch.members.push(conn_id.to_string());
                    let _ = ch.tx.send(ServerMessage::PresenceJoin {
                        channel: channel.to_string(),
                        user: user.clone(),
                    });
                }
            }
            if let Some(mut conn) = self.connections.get_mut(conn_id) {
                if !conn.channels.contains(&channel.to_string()) {
                    conn.channels.push(channel.to_string());
                }
            }
        }
    }

    pub async fn part_channel(&self, conn_id: &str, channel: &str) {
        let user_id = self.connections.get(conn_id).map(|c| c.user.user_id.clone());
        if let Some(uid) = user_id {
            if let Some(mut ch) = self.channels.get_mut(channel) {
                ch.members.retain(|id| id != conn_id);
                let _ = ch.tx.send(ServerMessage::PresencePart {
                    channel: channel.to_string(),
                    user_id: uid,
                });
            }
            if let Some(mut conn) = self.connections.get_mut(conn_id) {
                conn.channels.retain(|c| c != channel);
            }
        }
    }

    pub async fn broadcast_msg(
        &self,
        channel: &str,
        from: UserInfo,
        content: String,
        message_type: MessageKind,
    ) {
        if let Some(ch) = self.channels.get(channel) {
            let msg = ServerMessage::Msg {
                id: Uuid::new_v4(),
                channel: channel.to_string(),
                from,
                content,
                message_type,
                ts: Utc::now(),
            };
            let _ = ch.tx.send(msg);
        }
    }

    pub async fn set_topic(&self, channel: &str, topic: &str, set_by: &str) {
        if let Some(mut ch) = self.channels.get_mut(channel) {
            ch.topic = topic.to_string();
            let _ = ch.tx.send(ServerMessage::TopicUpdate {
                channel: channel.to_string(),
                topic: topic.to_string(),
                set_by: set_by.to_string(),
            });
        }
    }

    pub async fn channel_members(&self, channel: &str) -> Vec<UserInfo> {
        let conn_ids = self.channels.get(channel)
            .map(|ch| ch.members.clone())
            .unwrap_or_default();
        conn_ids.iter()
            .filter_map(|id| self.connections.get(id).map(|c| c.user.clone()))
            .collect()
    }

    pub async fn channel_topic(&self, channel: &str) -> String {
        self.channels.get(channel)
            .map(|ch| ch.topic.clone())
            .unwrap_or_default()
    }

    pub fn subscribe(&self, channel: &str) -> Option<broadcast::Receiver<ServerMessage>> {
        self.channels.get(channel).map(|ch| ch.tx.subscribe())
    }
}
