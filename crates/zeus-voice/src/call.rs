//! Call state machine and transcript management

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Call states
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallState {
    /// Call has been created but not yet initiated
    Initiated,
    /// Phone is ringing
    Ringing,
    /// Call has been answered
    Answered,
    /// Agent is speaking (TTS playing)
    Speaking,
    /// Agent is listening (STT active)
    Listening,
    /// Call is active (bidirectional)
    Active,
    /// Call has been completed normally
    Completed,
    /// Call failed
    Failed(String),
    /// No answer / busy
    NoAnswer,
    /// Call was cancelled
    Cancelled,
    /// Unknown state
    Unknown,
}

impl CallState {
    /// Convert Twilio call status to our state
    pub fn from_twilio_status(status: &str) -> Self {
        match status {
            "queued" | "initiated" => Self::Initiated,
            "ringing" => Self::Ringing,
            "in-progress" => Self::Active,
            "completed" => Self::Completed,
            "failed" => Self::Failed("Twilio reported failure".to_string()),
            "busy" | "no-answer" => Self::NoAnswer,
            "canceled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Convert Plivo call status to our state
    pub fn from_plivo_status(status: &str) -> Self {
        match status {
            "ringing" => Self::Ringing,
            "in-progress" => Self::Active,
            "answered" => Self::Answered,
            "completed" => Self::Completed,
            "busy" | "no-answer" | "timeout" => Self::NoAnswer,
            "failed" => Self::Failed("Plivo reported failure".to_string()),
            "cancel" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Check if the call is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed(_) | Self::NoAnswer | Self::Cancelled
        )
    }

    /// Check if the call is active (can send/receive audio)
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Answered | Self::Speaking | Self::Listening | Self::Active
        )
    }
}

/// A transcript entry from a call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// Who spoke: "agent" or "user"
    pub speaker: String,
    /// What was said
    pub text: String,
    /// When it was said
    pub timestamp: DateTime<Utc>,
}

impl TranscriptEntry {
    pub fn agent(text: &str) -> Self {
        Self {
            speaker: "agent".to_string(),
            text: text.to_string(),
            timestamp: Utc::now(),
        }
    }

    pub fn user(text: &str) -> Self {
        Self {
            speaker: "user".to_string(),
            text: text.to_string(),
            timestamp: Utc::now(),
        }
    }
}

/// Record of a voice call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    /// Call SID/ID from provider
    pub call_id: String,
    /// Phone number called
    pub to_number: String,
    /// Current state
    pub state: CallState,
    /// Call transcript
    pub transcript: Vec<TranscriptEntry>,
    /// When the call was initiated
    pub started_at: DateTime<Utc>,
    /// When the call ended (if terminal)
    pub ended_at: Option<DateTime<Utc>>,
    /// Duration in seconds
    pub duration_secs: Option<u64>,
}

impl CallRecord {
    pub fn new(call_id: String, to_number: String) -> Self {
        Self {
            call_id,
            to_number,
            state: CallState::Initiated,
            transcript: Vec::new(),
            started_at: Utc::now(),
            ended_at: None,
            duration_secs: None,
        }
    }

    /// Add a transcript entry
    pub fn add_transcript(&mut self, entry: TranscriptEntry) {
        self.transcript.push(entry);
    }

    /// Update call state
    pub fn update_state(&mut self, state: CallState) {
        if state.is_terminal() && self.ended_at.is_none() {
            self.ended_at = Some(Utc::now());
            let duration = Utc::now().signed_duration_since(self.started_at);
            self.duration_secs = Some(duration.num_seconds().max(0) as u64);
        }
        self.state = state;
    }
}

/// Manages active calls
pub struct CallManager {
    calls: Arc<RwLock<HashMap<String, CallRecord>>>,
}

impl CallManager {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new call
    pub async fn register_call(&self, call_id: String, to_number: String) -> CallRecord {
        let record = CallRecord::new(call_id.clone(), to_number);
        self.calls.write().await.insert(call_id, record.clone());
        record
    }

    /// Update call state
    pub async fn update_state(&self, call_id: &str, state: CallState) {
        if let Some(record) = self.calls.write().await.get_mut(call_id) {
            record.update_state(state);
        }
    }

    /// Add transcript entry
    pub async fn add_transcript(&self, call_id: &str, entry: TranscriptEntry) {
        if let Some(record) = self.calls.write().await.get_mut(call_id) {
            record.add_transcript(entry);
        }
    }

    /// Get a call record
    pub async fn get_call(&self, call_id: &str) -> Option<CallRecord> {
        self.calls.read().await.get(call_id).cloned()
    }

    /// List active calls
    pub async fn active_calls(&self) -> Vec<CallRecord> {
        self.calls
            .read()
            .await
            .values()
            .filter(|r| r.state.is_active())
            .cloned()
            .collect()
    }

    /// Remove completed calls older than a threshold
    pub async fn cleanup(&self, max_age_secs: u64) {
        let cutoff = Utc::now() - chrono::Duration::seconds(max_age_secs as i64);
        self.calls.write().await.retain(|_, r| {
            !r.state.is_terminal() || r.ended_at.map(|t| t > cutoff).unwrap_or(true)
        });
    }
}

impl Default for CallManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- CallState tests ----

    #[test]
    fn test_from_twilio_status_queued() {
        assert_eq!(
            CallState::from_twilio_status("queued"),
            CallState::Initiated
        );
    }

    #[test]
    fn test_from_twilio_status_initiated() {
        assert_eq!(
            CallState::from_twilio_status("initiated"),
            CallState::Initiated
        );
    }

    #[test]
    fn test_from_twilio_status_ringing() {
        assert_eq!(CallState::from_twilio_status("ringing"), CallState::Ringing);
    }

    #[test]
    fn test_from_twilio_status_in_progress() {
        assert_eq!(
            CallState::from_twilio_status("in-progress"),
            CallState::Active
        );
    }

    #[test]
    fn test_from_twilio_status_completed() {
        assert_eq!(
            CallState::from_twilio_status("completed"),
            CallState::Completed
        );
    }

    #[test]
    fn test_from_twilio_status_failed() {
        assert_eq!(
            CallState::from_twilio_status("failed"),
            CallState::Failed("Twilio reported failure".to_string())
        );
    }

    #[test]
    fn test_from_twilio_status_busy() {
        assert_eq!(CallState::from_twilio_status("busy"), CallState::NoAnswer);
    }

    #[test]
    fn test_from_twilio_status_no_answer() {
        assert_eq!(
            CallState::from_twilio_status("no-answer"),
            CallState::NoAnswer
        );
    }

    #[test]
    fn test_from_twilio_status_canceled() {
        assert_eq!(
            CallState::from_twilio_status("canceled"),
            CallState::Cancelled
        );
    }

    #[test]
    fn test_from_twilio_status_unknown() {
        assert_eq!(
            CallState::from_twilio_status("something-else"),
            CallState::Unknown
        );
    }

    #[test]
    fn test_is_terminal_completed() {
        assert!(CallState::Completed.is_terminal());
    }

    #[test]
    fn test_is_terminal_failed() {
        assert!(CallState::Failed("err".to_string()).is_terminal());
    }

    #[test]
    fn test_is_terminal_no_answer() {
        assert!(CallState::NoAnswer.is_terminal());
    }

    #[test]
    fn test_is_terminal_cancelled() {
        assert!(CallState::Cancelled.is_terminal());
    }

    #[test]
    fn test_is_not_terminal_active() {
        assert!(!CallState::Active.is_terminal());
        assert!(!CallState::Initiated.is_terminal());
        assert!(!CallState::Ringing.is_terminal());
        assert!(!CallState::Speaking.is_terminal());
        assert!(!CallState::Listening.is_terminal());
        assert!(!CallState::Unknown.is_terminal());
    }

    #[test]
    fn test_is_active_states() {
        assert!(CallState::Answered.is_active());
        assert!(CallState::Speaking.is_active());
        assert!(CallState::Listening.is_active());
        assert!(CallState::Active.is_active());
    }

    #[test]
    fn test_is_not_active_states() {
        assert!(!CallState::Initiated.is_active());
        assert!(!CallState::Ringing.is_active());
        assert!(!CallState::Completed.is_active());
        assert!(!CallState::Failed("err".to_string()).is_active());
        assert!(!CallState::NoAnswer.is_active());
        assert!(!CallState::Cancelled.is_active());
        assert!(!CallState::Unknown.is_active());
    }

    // ---- TranscriptEntry tests ----

    #[test]
    fn test_transcript_entry_agent() {
        let entry = TranscriptEntry::agent("Hello, this is Zeus");
        assert_eq!(entry.speaker, "agent");
        assert_eq!(entry.text, "Hello, this is Zeus");
    }

    #[test]
    fn test_transcript_entry_user() {
        let entry = TranscriptEntry::user("Hi, what do you need?");
        assert_eq!(entry.speaker, "user");
        assert_eq!(entry.text, "Hi, what do you need?");
    }

    // ---- CallRecord tests ----

    #[test]
    fn test_call_record_new() {
        let record = CallRecord::new("CA123".to_string(), "+15559876543".to_string());
        assert_eq!(record.call_id, "CA123");
        assert_eq!(record.to_number, "+15559876543");
        assert_eq!(record.state, CallState::Initiated);
        assert!(record.transcript.is_empty());
        assert!(record.ended_at.is_none());
        assert!(record.duration_secs.is_none());
    }

    #[test]
    fn test_call_record_add_transcript() {
        let mut record = CallRecord::new("CA123".to_string(), "+15559876543".to_string());
        record.add_transcript(TranscriptEntry::agent("Hello"));
        record.add_transcript(TranscriptEntry::user("Hi"));
        assert_eq!(record.transcript.len(), 2);
        assert_eq!(record.transcript[0].speaker, "agent");
        assert_eq!(record.transcript[1].speaker, "user");
    }

    #[test]
    fn test_call_record_state_transition_to_active() {
        let mut record = CallRecord::new("CA123".to_string(), "+15559876543".to_string());
        record.update_state(CallState::Ringing);
        assert_eq!(record.state, CallState::Ringing);
        assert!(record.ended_at.is_none());
        assert!(record.duration_secs.is_none());

        record.update_state(CallState::Active);
        assert_eq!(record.state, CallState::Active);
        assert!(record.ended_at.is_none());
    }

    #[test]
    fn test_call_record_state_transition_to_terminal() {
        let mut record = CallRecord::new("CA123".to_string(), "+15559876543".to_string());
        record.update_state(CallState::Active);
        assert!(record.ended_at.is_none());

        record.update_state(CallState::Completed);
        assert_eq!(record.state, CallState::Completed);
        assert!(record.ended_at.is_some());
        assert!(record.duration_secs.is_some());
    }

    #[test]
    fn test_call_record_terminal_only_sets_ended_at_once() {
        let mut record = CallRecord::new("CA123".to_string(), "+15559876543".to_string());
        record.update_state(CallState::Completed);
        let first_ended = record.ended_at;

        // Updating to another terminal state should not change ended_at
        record.update_state(CallState::Failed("late failure".to_string()));
        assert_eq!(record.ended_at, first_ended);
    }

    // ---- CallManager tests ----

    #[tokio::test]
    async fn test_call_manager_register_and_get() {
        let manager = CallManager::new();
        let record = manager
            .register_call("CA123".to_string(), "+15559876543".to_string())
            .await;
        assert_eq!(record.call_id, "CA123");

        let fetched = manager.get_call("CA123").await;
        assert!(fetched.is_some());
        assert_eq!(fetched.expect("operation should succeed").call_id, "CA123");
    }

    #[tokio::test]
    async fn test_call_manager_get_nonexistent() {
        let manager = CallManager::new();
        assert!(manager.get_call("NONEXISTENT").await.is_none());
    }

    #[tokio::test]
    async fn test_call_manager_update_state() {
        let manager = CallManager::new();
        manager
            .register_call("CA123".to_string(), "+15559876543".to_string())
            .await;

        manager.update_state("CA123", CallState::Active).await;

        let record = manager
            .get_call("CA123")
            .await
            .expect("async operation should succeed");
        assert_eq!(record.state, CallState::Active);
    }

    #[tokio::test]
    async fn test_call_manager_add_transcript() {
        let manager = CallManager::new();
        manager
            .register_call("CA123".to_string(), "+15559876543".to_string())
            .await;

        manager
            .add_transcript("CA123", TranscriptEntry::agent("Hello"))
            .await;
        manager
            .add_transcript("CA123", TranscriptEntry::user("Hi there"))
            .await;

        let record = manager
            .get_call("CA123")
            .await
            .expect("async operation should succeed");
        assert_eq!(record.transcript.len(), 2);
        assert_eq!(record.transcript[0].text, "Hello");
        assert_eq!(record.transcript[1].text, "Hi there");
    }

    #[tokio::test]
    async fn test_call_manager_active_calls() {
        let manager = CallManager::new();
        manager
            .register_call("CA1".to_string(), "+1111".to_string())
            .await;
        manager
            .register_call("CA2".to_string(), "+2222".to_string())
            .await;
        manager
            .register_call("CA3".to_string(), "+3333".to_string())
            .await;

        // CA1 is active, CA2 is completed, CA3 is still initiated
        manager.update_state("CA1", CallState::Active).await;
        manager.update_state("CA2", CallState::Completed).await;

        let active = manager.active_calls().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].call_id, "CA1");
    }

    #[tokio::test]
    async fn test_call_manager_cleanup() {
        let manager = CallManager::new();
        manager
            .register_call("CA1".to_string(), "+1111".to_string())
            .await;
        manager
            .register_call("CA2".to_string(), "+2222".to_string())
            .await;

        // Complete CA1
        manager.update_state("CA1", CallState::Completed).await;

        // Cleanup with 0 max_age should remove completed calls (ended_at is just now)
        // Use a large max_age so nothing gets cleaned up yet
        manager.cleanup(3600).await;
        assert!(manager.get_call("CA1").await.is_some());
        assert!(manager.get_call("CA2").await.is_some());
    }

    #[tokio::test]
    async fn test_call_manager_default() {
        let manager = CallManager::default();
        assert!(manager.active_calls().await.is_empty());
    }
}
