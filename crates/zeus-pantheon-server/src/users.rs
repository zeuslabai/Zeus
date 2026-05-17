use std::collections::HashMap;

use crate::protocol::{PresenceStatus, UserInfo};

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub user: UserInfo,
    pub presence: PresenceStatus,
    pub reserved_nick: Option<String>,
}

#[derive(Debug, Default)]
pub struct UserRegistry {
    users: HashMap<String, UserRecord>,
    nick_reservations: HashMap<String, String>,
}

impl UserRegistry {
    pub fn upsert(&mut self, user: UserInfo) {
        let entry = self.users.entry(user.user_id.clone()).or_insert(UserRecord {
            user: user.clone(),
            presence: PresenceStatus::Offline,
            reserved_nick: None,
        });
        entry.user = user;
        if entry.presence == PresenceStatus::Offline {
            entry.presence = PresenceStatus::Online;
        }
    }

    pub fn set_presence(&mut self, user_id: &str, presence: PresenceStatus) {
        if let Some(record) = self.users.get_mut(user_id) {
            record.presence = presence;
        }
    }

    pub fn reserve_nick(&mut self, user_id: &str, nick: &str) -> Result<(), String> {
        if let Some(existing) = self.nick_reservations.get(nick) {
            if existing != user_id {
                return Err("nick already reserved".into());
            }
        }
        self.nick_reservations.insert(nick.to_string(), user_id.to_string());
        if let Some(record) = self.users.get_mut(user_id) {
            record.reserved_nick = Some(nick.to_string());
        }
        Ok(())
    }

    pub fn release_nick(&mut self, nick: &str) {
        if let Some(user_id) = self.nick_reservations.remove(nick) {
            if let Some(record) = self.users.get_mut(&user_id) {
                record.reserved_nick = None;
            }
        }
    }

    pub fn get(&self, user_id: &str) -> Option<&UserRecord> {
        self.users.get(user_id)
    }

    pub fn all(&self) -> Vec<UserRecord> {
        self.users.values().cloned().collect()
    }
}
