use std::collections::HashMap;

use tokio::sync::broadcast;

use crate::protocol::ServerMessage;

const BROADCAST_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub topic: String,
    pub member_limit: Option<usize>,
    pub modes: Vec<String>,
}

#[derive(Debug)]
struct ChannelState {
    info: ChannelInfo,
    members: Vec<String>,
    tx: broadcast::Sender<ServerMessage>,
}

#[derive(Debug, Default)]
pub struct ChannelManager {
    channels: HashMap<String, ChannelState>,
}

impl ChannelManager {
    pub fn new(default_channels: &[String]) -> Self {
        let mut this = Self::default();
        for ch in default_channels {
            this.ensure_channel(ch);
        }
        this
    }

    pub fn ensure_channel(&mut self, channel: &str) {
        if self.channels.contains_key(channel) {
            return;
        }
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        self.channels.insert(
            channel.to_string(),
            ChannelState {
                info: ChannelInfo {
                    topic: String::new(),
                    member_limit: None,
                    modes: Vec::new(),
                },
                members: Vec::new(),
                tx,
            },
        );
    }

    pub fn join(&mut self, channel: &str, conn_id: String) -> Result<(), String> {
        self.ensure_channel(channel);
        let state = self.channels.get_mut(channel)
            .ok_or_else(|| format!("channel '{}' vanished after ensure", channel))?;
        if let Some(limit) = state.info.member_limit {
            if state.members.len() >= limit && !state.members.iter().any(|m| m == &conn_id) {
                return Err("channel full".into());
            }
        }
        if !state.members.contains(&conn_id) {
            state.members.push(conn_id);
        }
        Ok(())
    }

    pub fn part(&mut self, channel: &str, conn_id: &str) {
        if let Some(state) = self.channels.get_mut(channel) {
            state.members.retain(|m| m != conn_id);
        }
    }

    pub fn set_topic(&mut self, channel: &str, topic: impl Into<String>) {
        self.ensure_channel(channel);
        if let Some(state) = self.channels.get_mut(channel) {
            state.info.topic = topic.into();
        }
    }

    pub fn set_member_limit(&mut self, channel: &str, limit: Option<usize>) {
        self.ensure_channel(channel);
        if let Some(state) = self.channels.get_mut(channel) {
            state.info.member_limit = limit;
        }
    }

    pub fn set_modes(&mut self, channel: &str, modes: Vec<String>) {
        self.ensure_channel(channel);
        if let Some(state) = self.channels.get_mut(channel) {
            state.info.modes = modes;
        }
    }

    pub fn topic(&self, channel: &str) -> Option<String> {
        self.channels.get(channel).map(|c| c.info.topic.clone())
    }

    pub fn members(&self, channel: &str) -> Vec<String> {
        self.channels.get(channel).map(|c| c.members.clone()).unwrap_or_default()
    }

    pub fn subscribe(&self, channel: &str) -> Option<broadcast::Receiver<ServerMessage>> {
        self.channels.get(channel).map(|c| c.tx.subscribe())
    }

    pub fn sender(&self, channel: &str) -> Option<broadcast::Sender<ServerMessage>> {
        self.channels.get(channel).map(|c| c.tx.clone())
    }
}
