//! Zeus Athena - Documentation Engine
//!
//! This crate provides:
//! - Obsidian Markdown generation
//! - Apple Notes integration
//! - Automatic action logging
//! - Daily note generation
//! - Session summarization
//! - Cross-reference linking

pub mod config;
pub mod cross_ref;
pub mod notes;
pub mod obsidian;
pub mod summary;
pub use cross_ref::{
    BacklinkResult, CrossRefConfig, CrossRefLinker, CrossRefStats, DocEntry, DocLink, DocPath,
    LinkType,
};

pub use config::AthenaConfig;
pub use obsidian::{DailyStats, ObsidianWriter};
pub use summary::{CrossReferenceLinker, SessionSummary};

use chrono::{DateTime, Utc};
use zeus_core::Result;

/// The Athena documentation engine
pub struct Athena {
    obsidian: ObsidianWriter,
    config: AthenaConfig,
}

impl Athena {
    /// Create a new Athena instance
    pub fn new(config: AthenaConfig) -> Result<Self> {
        let obsidian = ObsidianWriter::new(&config.vault_path)?;

        Ok(Self { obsidian, config })
    }

    /// Get the current configuration
    pub fn config(&self) -> &AthenaConfig {
        &self.config
    }

    /// Log an action
    pub async fn log_action(&self, action: &ActionLog) -> Result<()> {
        // Write to daily note
        self.obsidian.append_to_daily(action).await?;

        // Write to session log if applicable
        if let Some(session_id) = &action.session_id {
            self.obsidian.append_to_session(session_id, action).await?;
        }

        Ok(())
    }

    /// Create or update a document
    pub async fn write_document(&self, path: &str, content: &str) -> Result<()> {
        self.obsidian.write(path, content).await
    }

    /// Append to a document
    pub async fn append_document(&self, path: &str, content: &str) -> Result<()> {
        self.obsidian.append(path, content).await
    }

    /// Create a daily note
    pub async fn create_daily_note(&self, date: DateTime<Utc>) -> Result<String> {
        self.obsidian.create_daily_note(date).await
    }

    /// Search documents
    pub async fn search(&self, query: &str) -> Result<Vec<DocumentMatch>> {
        self.obsidian.search(query).await
    }

    /// Generate and save a session summary
    pub async fn summarize_session(
        &self,
        session_id: &str,
        actions: &[ActionLog],
    ) -> Result<SessionSummary> {
        let summary = SessionSummary::from_actions(session_id, actions);
        let safe_id = crate::obsidian::sanitize_session_id(session_id);
        let path = format!("Sessions/{}-summary.md", safe_id);

        // Apply cross-reference linking and write the summary.
        // Falls back to a plain write if the linker can't be built.
        match self.create_linker().await {
            Ok(linker) => {
                self.write_linked_document(&path, &summary.to_markdown(), &linker)
                    .await?;
            }
            Err(_) => {
                self.obsidian.write(&path, &summary.to_markdown()).await?;
            }
        }

        Ok(summary)
    }

    /// Get a cross-reference linker pre-populated with known documents
    pub async fn create_linker(&self) -> Result<CrossReferenceLinker> {
        let mut linker = CrossReferenceLinker::new();

        // Use list_documents() — search("") is now rejected to prevent OOM (H4).
        let docs = self.obsidian.list_documents().await.unwrap_or_default();

        for path in docs {
            linker.add_document(path);
        }

        Ok(linker)
    }

    /// Write a document with automatic cross-reference linking
    pub async fn write_linked_document(
        &self,
        path: &str,
        content: &str,
        linker: &CrossReferenceLinker,
    ) -> Result<()> {
        let linked_content = linker.linkify(content);
        self.obsidian.write(path, &linked_content).await
    }
}

/// An action to log
#[derive(Debug, Clone)]
pub struct ActionLog {
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Session ID (if applicable)
    pub session_id: Option<String>,
    /// Action type
    pub action_type: ActionType,
    /// Action description
    pub description: String,
    /// Tool used (if any)
    pub tool: Option<String>,
    /// Result summary
    pub result: Option<String>,
    /// Duration in milliseconds
    pub duration_ms: Option<u64>,
}

impl ActionLog {
    /// Create a new action log
    pub fn new(action_type: ActionType, description: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            session_id: None,
            action_type,
            description: description.into(),
            tool: None,
            result: None,
            duration_ms: None,
        }
    }

    /// Set session ID
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Set tool
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tool = Some(tool.into());
        self
    }

    /// Set result
    pub fn with_result(mut self, result: impl Into<String>) -> Self {
        self.result = Some(result.into());
        self
    }

    /// Set duration
    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }
}

/// Action types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    /// Message received
    MessageReceived,
    /// Task planned
    TaskPlanned,
    /// Tool executed
    ToolExecuted,
    /// Response sent
    ResponseSent,
    /// Error occurred
    Error,
    /// System event
    System,
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageReceived => write!(f, "📥 Message"),
            Self::TaskPlanned => write!(f, "📋 Planned"),
            Self::ToolExecuted => write!(f, "🔧 Tool"),
            Self::ResponseSent => write!(f, "📤 Response"),
            Self::Error => write!(f, "❌ Error"),
            Self::System => write!(f, "⚙️ System"),
        }
    }
}

/// A document match from search
#[derive(Debug)]
pub struct DocumentMatch {
    /// Document path
    pub path: String,
    /// Match context
    pub context: String,
    /// Line number
    pub line: usize,
}
