//! Session management

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Session manager
pub struct SessionManager {
    sessions: HashMap<String, SessionInfo>,
    active_session: Option<String>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            active_session: None,
        }
    }

    /// Create a new session
    pub fn create_session(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let info = SessionInfo {
            id: id.clone(),
            created_at: Utc::now(),
            last_activity: Utc::now(),
            message_count: 0,
        };
        self.sessions.insert(id.clone(), info);
        self.active_session = Some(id.clone());
        id
    }

    /// Get current session
    pub fn current(&self) -> Option<&SessionInfo> {
        self.active_session
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    /// Switch to a session
    pub fn switch_to(&mut self, id: &str) -> bool {
        if self.sessions.contains_key(id) {
            self.active_session = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// List all sessions
    pub fn list(&self) -> Vec<&SessionInfo> {
        self.sessions.values().collect()
    }

    /// Record activity in current session
    pub fn record_activity(&mut self) {
        if let Some(id) = &self.active_session
            && let Some(session) = self.sessions.get_mut(id)
        {
            session.last_activity = Utc::now();
            session.message_count += 1;
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session ID
    pub id: String,
    /// When session was created
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
    /// Number of messages
    pub message_count: usize,
}
