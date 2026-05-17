use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::protocol::{MessageKind, ServerMessage, UserInfo};

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: Uuid,
    pub channel: String,
    pub from: UserInfo,
    pub content: String,
    pub message_type: MessageKind,
    pub ts: DateTime<Utc>,
}

#[derive(Debug)]
pub struct MessageStore {
    limit: usize,
    histories: HashMap<String, VecDeque<StoredMessage>>,
}

impl MessageStore {
    pub fn new(limit: usize) -> Self {
        Self { limit, histories: HashMap::new() }
    }

    pub fn push(&mut self, channel: &str, from: UserInfo, content: String, message_type: MessageKind) -> StoredMessage {
        let msg = StoredMessage {
            id: Uuid::new_v4(),
            channel: channel.to_string(),
            from,
            content,
            message_type,
            ts: Utc::now(),
        };
        let history = self.histories.entry(channel.to_string()).or_default();
        history.push_back(msg.clone());
        while history.len() > self.limit {
            history.pop_front();
        }
        msg
    }

    pub fn history(&self, channel: &str) -> Vec<StoredMessage> {
        self.histories
            .get(channel)
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn search(&self, channel: &str, query: &str) -> Vec<StoredMessage> {
        let q = query.to_lowercase();
        self.history(channel)
            .into_iter()
            .filter(|m| m.content.to_lowercase().contains(&q) || m.from.display_name.to_lowercase().contains(&q))
            .collect()
    }

    pub fn to_server_message(msg: &StoredMessage) -> ServerMessage {
        ServerMessage::Msg {
            id: msg.id,
            channel: msg.channel.clone(),
            from: msg.from.clone(),
            content: msg.content.clone(),
            message_type: msg.message_type.clone(),
            ts: msg.ts,
        }
    }
}
