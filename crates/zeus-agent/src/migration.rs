//! Data migration/import engine for Zeus
//!
//! Supports importing data from:
//! - OpenClaw (SKILL.md + sessions + memory)
//! - ChatGPT export (conversations.json)
//! - Claude export (conversations.json with Claude format)
//! - Generic JSON/JSONL message files

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info};
use uuid::Uuid;
use zeus_core::{Error, Message, Result, Role};

// ============================================================================
// Types
// ============================================================================

/// Source format for imported data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    /// OpenClaw format (SKILL.md files, session JSON, memory directory)
    OpenClaw,
    /// ChatGPT data export (conversations.json)
    ChatGPT,
    /// Claude data export (conversations.json with Claude-specific format)
    ClaudeExport,
    /// Generic JSON or JSONL message files
    Generic,
}

impl std::fmt::Display for ImportSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportSource::OpenClaw => write!(f, "OpenClaw"),
            ImportSource::ChatGPT => write!(f, "ChatGPT"),
            ImportSource::ClaudeExport => write!(f, "Claude Export"),
            ImportSource::Generic => write!(f, "Generic"),
        }
    }
}

/// Results from an import operation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportResult {
    /// Number of sessions imported
    pub sessions_imported: u32,
    /// Number of memory entries imported
    pub memories_imported: u32,
    /// Number of skills imported
    pub skills_imported: u32,
    /// Errors encountered during import (non-fatal)
    pub errors: Vec<String>,
    /// Warnings generated during import
    pub warnings: Vec<String>,
    /// The detected source format
    pub source: Option<ImportSource>,
}

impl ImportResult {
    /// Merge another ImportResult into this one
    pub fn merge(&mut self, other: ImportResult) {
        self.sessions_imported += other.sessions_imported;
        self.memories_imported += other.memories_imported;
        self.skills_imported += other.skills_imported;
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }

    /// Total number of items imported
    pub fn total_imported(&self) -> u32 {
        self.sessions_imported + self.memories_imported + self.skills_imported
    }

    /// Whether the import had any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Format a human-readable summary
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.sessions_imported > 0 {
            parts.push(format!("{} sessions", self.sessions_imported));
        }
        if self.memories_imported > 0 {
            parts.push(format!("{} memories", self.memories_imported));
        }
        if self.skills_imported > 0 {
            parts.push(format!("{} skills", self.skills_imported));
        }
        if parts.is_empty() {
            "Nothing imported".to_string()
        } else {
            format!("Imported: {}", parts.join(", "))
        }
    }
}

// ============================================================================
// JSONL Session Entry (matches zeus-session format)
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct SessionEntry {
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

// ============================================================================
// Generic message format
// ============================================================================

#[derive(Debug, Deserialize)]
struct GenericMessage {
    role: Option<String>,
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    sender: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
}

impl GenericMessage {
    /// Extract the role, defaulting to "user"
    fn effective_role(&self) -> Role {
        let role_str = self
            .role
            .as_deref()
            .or(self.sender.as_deref())
            .unwrap_or("user");
        match role_str.to_lowercase().as_str() {
            "assistant" | "bot" | "ai" | "system_assistant" => Role::Assistant,
            "system" => Role::System,
            "tool" | "function" => Role::Tool,
            _ => Role::User,
        }
    }

    /// Extract the content from whichever field is available
    fn effective_content(&self) -> String {
        self.content
            .as_deref()
            .or(self.text.as_deref())
            .or(self.message.as_deref())
            .unwrap_or("")
            .to_string()
    }
}

// ============================================================================
// MigrationEngine
// ============================================================================

/// Engine for importing data from external sources into Zeus format
pub struct MigrationEngine {
    /// Directory to write imported sessions to
    sessions_dir: PathBuf,
    /// Workspace directory for memory files
    workspace_dir: PathBuf,
}

impl MigrationEngine {
    /// Create a new MigrationEngine
    pub fn new(sessions_dir: PathBuf, workspace_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            workspace_dir,
        }
    }

    /// Auto-detect the source format from the given path
    pub fn detect_source(path: &Path) -> ImportSource {
        if !path.exists() {
            return ImportSource::Generic;
        }

        // Check for OpenClaw indicators: SKILL.md files
        if path.is_dir() {
            // Look for SKILL.md at top level or in subdirs
            if path.join("SKILL.md").exists() {
                return ImportSource::OpenClaw;
            }
            // Check subdirectories for SKILL.md
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() && entry.path().join("SKILL.md").exists() {
                        return ImportSource::OpenClaw;
                    }
                }
            }
            // Check for ChatGPT conversations.json
            if path.join("conversations.json").exists() {
                // Try to distinguish ChatGPT from Claude by peeking at the file
                let conv_path = path.join("conversations.json");
                if let Ok(content) = std::fs::read_to_string(&conv_path) {
                    // ChatGPT exports have "mapping" field with node structure
                    if content.contains("\"mapping\"") {
                        return ImportSource::ChatGPT;
                    }
                    // Claude exports have "chat_messages" field
                    if content.contains("\"chat_messages\"") {
                        return ImportSource::ClaudeExport;
                    }
                    // Default to ChatGPT if conversations.json exists
                    return ImportSource::ChatGPT;
                }
            }
        } else if path.is_file() {
            // Single file: check extension and content
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if (ext == "json" || ext == "jsonl")
                && let Ok(content) = std::fs::read_to_string(path)
            {
                if content.contains("\"mapping\"") && content.contains("\"title\"") {
                    return ImportSource::ChatGPT;
                }
                if content.contains("\"chat_messages\"") {
                    return ImportSource::ClaudeExport;
                }
            }
        }

        ImportSource::Generic
    }

    /// Import data from an OpenClaw-format directory
    pub async fn import_openclaw(&self, path: &Path) -> Result<ImportResult> {
        let mut result = ImportResult {
            source: Some(ImportSource::OpenClaw),
            ..Default::default()
        };

        if !path.is_dir() {
            return Err(Error::Config(format!(
                "OpenClaw import path is not a directory: {}",
                path.display()
            )));
        }

        // 1. Scan for SKILL.md files -> import as skills
        let skills_target = self.workspace_dir.join("skills");
        self.scan_and_import_skills(path, &skills_target, &mut result)
            .await;

        // 2. Scan for session JSON files -> convert to Zeus JSONL
        self.scan_and_import_sessions(path, &mut result).await;

        // 3. Scan for memory directory -> copy to workspace/memory/
        let memory_source = path.join("memory");
        if memory_source.is_dir() {
            self.import_memory_dir(&memory_source, &mut result).await;
        }

        // 4. Discover OpenClaw extensions in the path (or ~/openclaw/extensions/)
        let extensions_dir = if path.join("extensions").is_dir() {
            path.join("extensions")
        } else {
            path.to_path_buf()
        };
        self.import_openclaw_extensions(&extensions_dir, &mut result)
            .await;

        info!("OpenClaw import complete: {}", result.summary());
        Ok(result)
    }

    /// Discover and register OpenClaw extensions from a directory.
    async fn import_openclaw_extensions(&self, extensions_dir: &Path, result: &mut ImportResult) {
        use zeus_extensions::openclaw::discover_openclaw_extensions;

        match discover_openclaw_extensions(extensions_dir) {
            Ok(discovered) => {
                let valid_count = discovered.iter().filter(|d| d.has_entry_point).count();
                if valid_count > 0 {
                    info!(
                        "Found {} OpenClaw extensions ({} with entry points)",
                        discovered.len(),
                        valid_count
                    );
                    // Copy extension manifests to workspace for reference
                    let target = self.workspace_dir.join("extensions");
                    if let Err(e) = std::fs::create_dir_all(&target) {
                        result
                            .errors
                            .push(format!("Failed to create extensions dir: {}", e));
                        return;
                    }

                    for disc in &discovered {
                        if !disc.has_entry_point {
                            continue;
                        }
                        let manifest_src = disc.path.join("openclaw.plugin.json");
                        let manifest_dst = target.join(format!("{}.json", disc.manifest.id));
                        if let Err(e) = std::fs::copy(&manifest_src, &manifest_dst) {
                            result.warnings.push(format!(
                                "Could not copy manifest for '{}': {}",
                                disc.manifest.id, e
                            ));
                        } else {
                            result.skills_imported += 1;
                        }
                    }
                }
            }
            Err(_) => {
                // Not an extensions directory — that's fine, skip silently
            }
        }
    }

    /// Import data from a ChatGPT export
    pub async fn import_chatgpt(&self, path: &Path) -> Result<ImportResult> {
        let mut result = ImportResult {
            source: Some(ImportSource::ChatGPT),
            ..Default::default()
        };

        let conversations_path = if path.is_dir() {
            path.join("conversations.json")
        } else {
            path.to_path_buf()
        };

        if !conversations_path.exists() {
            return Err(Error::Config(format!(
                "conversations.json not found at: {}",
                conversations_path.display()
            )));
        }

        let content = std::fs::read_to_string(&conversations_path)?;
        let conversations: Vec<serde_json::Value> = serde_json::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse conversations.json: {}", e)))?;

        std::fs::create_dir_all(&self.sessions_dir)?;

        for conv in &conversations {
            match self.convert_chatgpt_conversation(conv).await {
                Ok(()) => result.sessions_imported += 1,
                Err(e) => result
                    .errors
                    .push(format!("Failed to convert conversation: {}", e)),
            }
        }

        info!("ChatGPT import complete: {}", result.summary());
        Ok(result)
    }

    /// Import from generic JSON or JSONL files
    pub async fn import_generic(&self, path: &Path) -> Result<ImportResult> {
        let mut result = ImportResult {
            source: Some(ImportSource::Generic),
            ..Default::default()
        };

        if path.is_dir() {
            // Import all .json and .jsonl files in the directory
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_file() {
                        let ext = entry_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("");
                        if ext == "json" || ext == "jsonl" {
                            match self.import_generic_file(&entry_path).await {
                                Ok(count) => result.sessions_imported += count,
                                Err(e) => result.errors.push(format!(
                                    "Failed to import {}: {}",
                                    entry_path.display(),
                                    e
                                )),
                            }
                        }
                    }
                }
            }
        } else if path.is_file() {
            match self.import_generic_file(path).await {
                Ok(count) => result.sessions_imported += count,
                Err(e) => result
                    .errors
                    .push(format!("Failed to import {}: {}", path.display(), e)),
            }
        } else {
            return Err(Error::Config(format!(
                "Import path does not exist: {}",
                path.display()
            )));
        }

        info!("Generic import complete: {}", result.summary());
        Ok(result)
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    /// Scan a directory tree for SKILL.md files and copy them into the skills target
    async fn scan_and_import_skills(
        &self,
        source: &Path,
        target: &Path,
        result: &mut ImportResult,
    ) {
        // Check top level
        if source.join("SKILL.md").exists() {
            let name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("imported-skill");
            match self.copy_skill(source, target, name) {
                Ok(()) => result.skills_imported += 1,
                Err(e) => result
                    .errors
                    .push(format!("Failed to import skill '{}': {}", name, e)),
            }
        }

        // Check subdirectories
        if let Ok(entries) = std::fs::read_dir(source) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() && entry_path.join("SKILL.md").exists() {
                    let name = entry_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("imported-skill");
                    match self.copy_skill(&entry_path, target, name) {
                        Ok(()) => result.skills_imported += 1,
                        Err(e) => result
                            .errors
                            .push(format!("Failed to import skill '{}': {}", name, e)),
                    }
                }
            }
        }
    }

    /// Copy a skill directory to the target skills directory
    fn copy_skill(&self, source_dir: &Path, target_base: &Path, name: &str) -> Result<()> {
        let target_dir = target_base.join(name);
        std::fs::create_dir_all(&target_dir)?;

        // Copy SKILL.md
        let skill_src = source_dir.join("SKILL.md");
        let skill_dst = target_dir.join("SKILL.md");
        std::fs::copy(&skill_src, &skill_dst)?;
        debug!(
            "Imported skill: {} -> {}",
            skill_src.display(),
            skill_dst.display()
        );

        // Copy other files in the skill directory (scripts, configs, etc.)
        if let Ok(entries) = std::fs::read_dir(source_dir) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_file() && entry_path.file_name().unwrap_or_default() != "SKILL.md"
                {
                    let fname = match entry_path.file_name() {
                        Some(f) => f,
                        None => continue, // skip entries without a filename
                    };
                    let dst = target_dir.join(fname);
                    std::fs::copy(&entry_path, &dst)?;
                }
            }
        }

        Ok(())
    }

    /// Scan for session files and convert them to Zeus JSONL format
    async fn scan_and_import_sessions(&self, source: &Path, result: &mut ImportResult) {
        // Look for sessions/ directory or session*.json files
        let session_dir = source.join("sessions");
        let search_dir = if session_dir.is_dir() {
            &session_dir
        } else {
            source
        };

        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    let ext = entry_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if ext == "json" || ext == "jsonl" {
                        match self.convert_openclaw_session(&entry_path).await {
                            Ok(true) => result.sessions_imported += 1,
                            Ok(false) => {
                                // Not a session file, skip silently
                            }
                            Err(e) => result.errors.push(format!(
                                "Failed to import session {}: {}",
                                entry_path.display(),
                                e
                            )),
                        }
                    }
                }
            }
        }
    }

    /// Convert an OpenClaw session file to Zeus JSONL format
    async fn convert_openclaw_session(&self, path: &Path) -> Result<bool> {
        let content = std::fs::read_to_string(path)?;
        let messages = self.parse_messages_from_json(&content)?;

        if messages.is_empty() {
            return Ok(false);
        }

        self.write_session_jsonl(&messages).await?;
        Ok(true)
    }

    /// Import a memory directory by copying markdown files
    async fn import_memory_dir(&self, source: &Path, result: &mut ImportResult) {
        let target = self.workspace_dir.join("memory");
        if let Err(e) = std::fs::create_dir_all(&target) {
            result
                .errors
                .push(format!("Failed to create memory dir: {}", e));
            return;
        }

        if let Ok(entries) = std::fs::read_dir(source) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    let filename = entry_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let dst = target.join(&filename);

                    // Append to existing file or copy new
                    if dst.exists() {
                        match std::fs::read_to_string(&entry_path) {
                            Ok(new_content) => {
                                match std::fs::OpenOptions::new().append(true).open(&dst) {
                                    Ok(mut file) => {
                                        use std::io::Write;
                                        let _ = writeln!(file, "\n---\n{}", new_content);
                                        result.memories_imported += 1;
                                    }
                                    Err(e) => result.errors.push(format!(
                                        "Failed to append to {}: {}",
                                        dst.display(),
                                        e
                                    )),
                                }
                            }
                            Err(e) => result.errors.push(format!(
                                "Failed to read {}: {}",
                                entry_path.display(),
                                e
                            )),
                        }
                    } else {
                        match std::fs::copy(&entry_path, &dst) {
                            Ok(_) => result.memories_imported += 1,
                            Err(e) => result.errors.push(format!(
                                "Failed to copy {} -> {}: {}",
                                entry_path.display(),
                                dst.display(),
                                e
                            )),
                        }
                    }
                }
            }
        }
    }

    /// Convert a single ChatGPT conversation to Zeus JSONL
    async fn convert_chatgpt_conversation(&self, conv: &serde_json::Value) -> Result<()> {
        let mapping = conv
            .get("mapping")
            .and_then(|m| m.as_object())
            .ok_or_else(|| Error::Config("Conversation missing 'mapping' field".to_string()))?;

        // Build an ordered list of messages from the mapping graph
        let mut messages = Vec::new();

        // Collect all nodes with message content
        let mut nodes: Vec<(&str, &serde_json::Value)> = mapping
            .iter()
            .filter_map(|(id, node)| {
                node.get("message").and_then(|m| {
                    if m.is_null() {
                        None
                    } else {
                        Some((id.as_str(), m))
                    }
                })
            })
            .collect();

        // Sort by create_time if available
        nodes.sort_by(|a, b| {
            let time_a =
                a.1.get("create_time")
                    .and_then(|t| t.as_f64())
                    .unwrap_or(0.0);
            let time_b =
                b.1.get("create_time")
                    .and_then(|t| t.as_f64())
                    .unwrap_or(0.0);
            time_a
                .partial_cmp(&time_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (_id, msg) in &nodes {
            let role_str = msg
                .get("author")
                .and_then(|a| a.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("user");

            let content = msg
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();

            if content.is_empty() {
                continue;
            }

            let role = match role_str {
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => Role::User,
            };

            let timestamp = msg
                .get("create_time")
                .and_then(|t| t.as_f64())
                .map(|ts| {
                    DateTime::from_timestamp(ts as i64, ((ts.fract()) * 1_000_000_000.0) as u32)
                        .unwrap_or_else(Utc::now)
                })
                .unwrap_or_else(Utc::now);

            messages.push(Message {
                role,
                direction: zeus_core::detect_direction(&content), channel_source: None, compaction_hint: Default::default(),
                content,
                tool_calls: vec![],
                tool_results: vec![],
                timestamp,
                attachments: vec![],
                message_id: Some(uuid::Uuid::new_v4().to_string()),
                parent_id: None,
                thread_id: None,
            });
        }

        if messages.is_empty() {
            return Err(Error::Config("Conversation has no messages".to_string()));
        }

        self.write_session_jsonl(&messages).await
    }

    /// Import a generic JSON or JSONL file
    async fn import_generic_file(&self, path: &Path) -> Result<u32> {
        let content = std::fs::read_to_string(path)?;
        let mut count = 0u32;

        // Try parsing as JSONL first (one JSON object per line)
        let is_jsonl = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "jsonl")
            .unwrap_or(false);

        if is_jsonl
            || content.lines().all(|line| {
                line.trim().is_empty()
                    || serde_json::from_str::<serde_json::Value>(line.trim()).is_ok()
            })
        {
            let messages = self.parse_messages_from_jsonl(&content)?;
            if !messages.is_empty() {
                self.write_session_jsonl(&messages).await?;
                return Ok(1);
            }
        }

        // Try parsing as a JSON array of messages
        let messages = self.parse_messages_from_json(&content)?;
        if !messages.is_empty() {
            self.write_session_jsonl(&messages).await?;
            count += 1;
        }

        Ok(count)
    }

    /// Parse messages from a JSON string (array or object with messages field)
    fn parse_messages_from_json(&self, content: &str) -> Result<Vec<Message>> {
        let value: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| Error::Config(format!("Invalid JSON: {}", e)))?;

        let msg_array = if let Some(arr) = value.as_array() {
            arr.clone()
        } else if let Some(msgs) = value.get("messages").and_then(|m| m.as_array()) {
            msgs.clone()
        } else if let Some(conv) = value.get("conversation").and_then(|c| c.as_array()) {
            conv.clone()
        } else {
            return Ok(Vec::new());
        };

        let mut messages = Vec::new();
        for item in &msg_array {
            if let Ok(gm) = serde_json::from_value::<GenericMessage>(item.clone()) {
                let content = gm.effective_content();
                if content.is_empty() {
                    continue;
                }
                let timestamp = gm
                    .timestamp
                    .as_deref()
                    .and_then(|t| t.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);

                messages.push(Message {
                    role: gm.effective_role(),
                    direction: zeus_core::detect_direction(&content), channel_source: None, compaction_hint: Default::default(),
                    content,
                    tool_calls: vec![],
                    tool_results: vec![],
                    timestamp,
                    attachments: vec![],
                    message_id: Some(uuid::Uuid::new_v4().to_string()),
                    parent_id: None,
                    thread_id: None,
                });
            }
        }

        Ok(messages)
    }

    /// Parse messages from JSONL content (one JSON object per line)
    fn parse_messages_from_jsonl(&self, content: &str) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(gm) = serde_json::from_str::<GenericMessage>(line) {
                let content = gm.effective_content();
                if content.is_empty() {
                    continue;
                }
                let timestamp = gm
                    .timestamp
                    .as_deref()
                    .and_then(|t| t.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);

                messages.push(Message {
                    role: gm.effective_role(),
                    direction: zeus_core::detect_direction(&content), channel_source: None, compaction_hint: Default::default(),
                    content,
                    tool_calls: vec![],
                    tool_results: vec![],
                    timestamp,
                    attachments: vec![],
                    message_id: Some(uuid::Uuid::new_v4().to_string()),
                    parent_id: None,
                    thread_id: None,
                });
            }
        }

        Ok(messages)
    }

    /// Write a list of messages as a Zeus session JSONL file
    async fn write_session_jsonl(&self, messages: &[Message]) -> Result<()> {
        std::fs::create_dir_all(&self.sessions_dir)?;

        let session_id = Uuid::new_v4().to_string();
        let path = self.sessions_dir.join(format!("{}.jsonl", session_id));

        let created = messages
            .first()
            .map(|m| m.timestamp)
            .unwrap_or_else(Utc::now);

        let mut content = String::new();

        // Write session_start entry
        let start_entry = SessionEntry {
            entry_type: "session_start".to_string(),
            data: serde_json::json!({
                "id": session_id,
                "created": created.to_rfc3339(),
            }),
        };
        content.push_str(&serde_json::to_string(&start_entry)?);
        content.push('\n');

        // Write each message
        for msg in messages {
            let msg_entry = SessionEntry {
                entry_type: "message".to_string(),
                data: serde_json::to_value(msg)?,
            };
            content.push_str(&serde_json::to_string(&msg_entry)?);
            content.push('\n');
        }

        std::fs::write(&path, content)?;
        debug!("Wrote imported session: {}", path.display());

        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_import_source_display() {
        assert_eq!(ImportSource::OpenClaw.to_string(), "OpenClaw");
        assert_eq!(ImportSource::ChatGPT.to_string(), "ChatGPT");
        assert_eq!(ImportSource::ClaudeExport.to_string(), "Claude Export");
        assert_eq!(ImportSource::Generic.to_string(), "Generic");
    }

    #[test]
    fn test_import_result_defaults() {
        let result = ImportResult::default();
        assert_eq!(result.sessions_imported, 0);
        assert_eq!(result.memories_imported, 0);
        assert_eq!(result.skills_imported, 0);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert!(result.source.is_none());
        assert_eq!(result.total_imported(), 0);
        assert!(!result.has_errors());
    }

    #[test]
    fn test_import_result_merge() {
        let mut a = ImportResult {
            sessions_imported: 2,
            memories_imported: 1,
            skills_imported: 3,
            errors: vec!["error1".to_string()],
            warnings: vec![],
            source: Some(ImportSource::OpenClaw),
        };
        let b = ImportResult {
            sessions_imported: 1,
            memories_imported: 4,
            skills_imported: 0,
            errors: vec!["error2".to_string()],
            warnings: vec!["warn1".to_string()],
            source: Some(ImportSource::Generic),
        };
        a.merge(b);

        assert_eq!(a.sessions_imported, 3);
        assert_eq!(a.memories_imported, 5);
        assert_eq!(a.skills_imported, 3);
        assert_eq!(a.errors.len(), 2);
        assert_eq!(a.warnings.len(), 1);
        assert_eq!(a.total_imported(), 11);
        assert!(a.has_errors());
    }

    #[test]
    fn test_import_result_summary() {
        let result = ImportResult {
            sessions_imported: 5,
            memories_imported: 3,
            skills_imported: 2,
            ..Default::default()
        };
        let summary = result.summary();
        assert!(summary.contains("5 sessions"));
        assert!(summary.contains("3 memories"));
        assert!(summary.contains("2 skills"));

        let empty = ImportResult::default();
        assert_eq!(empty.summary(), "Nothing imported");
    }

    #[test]
    fn test_detect_source_openclaw() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let skill_dir = tmp.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).expect("Failed to create skill directory");
        std::fs::write(skill_dir.join("SKILL.md"), "# Test Skill\nA test.")
            .expect("Failed to write SKILL.md");

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::OpenClaw);
    }

    #[test]
    fn test_detect_source_chatgpt() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let content = r#"[{"title": "Hello", "mapping": {"node1": {}}}]"#;
        std::fs::write(tmp.path().join("conversations.json"), content)
            .expect("Failed to write conversations.json");

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::ChatGPT);
    }

    #[test]
    fn test_detect_source_claude_export() {
        let tmp = TempDir::new().unwrap();
        let content = r#"[{"chat_messages": []}]"#;
        std::fs::write(tmp.path().join("conversations.json"), content).unwrap();

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::ClaudeExport);
    }

    #[test]
    fn test_detect_source_generic_fallback() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("data.json"),
            r#"[{"role": "user", "content": "hi"}]"#,
        )
        .unwrap();

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::Generic);
    }

    #[test]
    fn test_detect_source_nonexistent_path() {
        let detected = MigrationEngine::detect_source(Path::new("/nonexistent/path/xyz"));
        assert_eq!(detected, ImportSource::Generic);
    }

    #[tokio::test]
    async fn test_import_generic_with_simple_messages() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let messages_file = tmp.path().join("messages.json");
        let content = r#"[
            {"role": "user", "content": "Hello there!"},
            {"role": "assistant", "content": "Hi! How can I help?"},
            {"role": "user", "content": "Tell me a joke"}
        ]"#;
        std::fs::write(&messages_file, content).unwrap();

        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);
        let result = engine.import_generic(&messages_file).await.unwrap();

        assert_eq!(result.sessions_imported, 1);
        assert!(!result.has_errors());

        // Verify session file was created
        let session_files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(session_files.len(), 1);

        // Verify content
        let session_content = std::fs::read_to_string(session_files[0].path()).unwrap();
        assert!(session_content.contains("session_start"));
        assert!(session_content.contains("Hello there!"));
        assert!(session_content.contains("Hi! How can I help?"));
    }

    #[tokio::test]
    async fn test_import_generic_with_jsonl() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let messages_file = tmp.path().join("messages.jsonl");
        let content = r#"{"role": "user", "content": "Line 1"}
{"role": "assistant", "content": "Response 1"}
{"role": "user", "text": "Using text field"}
"#;
        std::fs::write(&messages_file, content).unwrap();

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine.import_generic(&messages_file).await.unwrap();

        assert_eq!(result.sessions_imported, 1);
        assert!(!result.has_errors());
    }

    #[tokio::test]
    async fn test_import_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let empty_dir = tmp.path().join("empty");
        std::fs::create_dir_all(&empty_dir).unwrap();

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine.import_generic(&empty_dir).await.unwrap();

        assert_eq!(result.sessions_imported, 0);
        assert!(!result.has_errors());
    }

    #[tokio::test]
    async fn test_import_generic_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let bad_file = tmp.path().join("bad.json");
        std::fs::write(&bad_file, "this is not json at all{{{").unwrap();

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine.import_generic(&bad_file).await.unwrap();

        // Should have errors since JSON parsing fails
        assert!(result.has_errors());
        assert_eq!(result.sessions_imported, 0);
    }

    #[tokio::test]
    async fn test_import_openclaw_with_skills_and_memory() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        // Set up OpenClaw-like directory structure
        let openclaw_dir = tmp.path().join("openclaw");
        let skill_dir = openclaw_dir.join("git-helper");
        let memory_dir = openclaw_dir.join("memory");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::create_dir_all(&memory_dir).unwrap();

        // Create SKILL.md
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "# Git Helper\nA helpful git skill.\n\n## Tools\n- status: Show git status\n",
        )
        .unwrap();

        // Create memory file
        std::fs::write(memory_dir.join("MEMORY.md"), "- User prefers dark mode\n").unwrap();

        let engine = MigrationEngine::new(sessions_dir, workspace_dir.clone());
        let result = engine.import_openclaw(&openclaw_dir).await.unwrap();

        assert_eq!(result.skills_imported, 1);
        assert_eq!(result.memories_imported, 1);

        // Verify skill was copied
        assert!(
            workspace_dir
                .join("skills")
                .join("git-helper")
                .join("SKILL.md")
                .exists()
        );

        // Verify memory was copied
        assert!(workspace_dir.join("memory").join("MEMORY.md").exists());
    }

    #[tokio::test]
    async fn test_import_chatgpt_conversations() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let chatgpt_dir = tmp.path().join("chatgpt");
        std::fs::create_dir_all(&chatgpt_dir).unwrap();

        let content = r#"[
            {
                "title": "Test Conversation",
                "create_time": 1700000000.0,
                "mapping": {
                    "node1": {
                        "message": {
                            "author": {"role": "user"},
                            "content": {"parts": ["Hello ChatGPT"]},
                            "create_time": 1700000001.0
                        }
                    },
                    "node2": {
                        "message": {
                            "author": {"role": "assistant"},
                            "content": {"parts": ["Hello! How can I help you today?"]},
                            "create_time": 1700000002.0
                        }
                    },
                    "node0": {
                        "message": null
                    }
                }
            }
        ]"#;
        std::fs::write(chatgpt_dir.join("conversations.json"), content).unwrap();

        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);
        let result = engine.import_chatgpt(&chatgpt_dir).await.unwrap();

        assert_eq!(result.sessions_imported, 1);
        assert!(!result.has_errors());

        // Verify session was written
        let session_files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(session_files.len(), 1);

        let session_content = std::fs::read_to_string(session_files[0].path()).unwrap();
        assert!(session_content.contains("Hello ChatGPT"));
        assert!(session_content.contains("Hello! How can I help you today?"));
    }

    #[test]
    fn test_generic_message_effective_role() {
        let user = GenericMessage {
            role: Some("user".to_string()),
            content: None,
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(user.effective_role(), Role::User);

        let bot = GenericMessage {
            role: Some("bot".to_string()),
            content: None,
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(bot.effective_role(), Role::Assistant);

        let sender_assistant = GenericMessage {
            role: None,
            content: None,
            text: None,
            message: None,
            sender: Some("ai".to_string()),
            timestamp: None,
        };
        assert_eq!(sender_assistant.effective_role(), Role::Assistant);

        let default_user = GenericMessage {
            role: None,
            content: None,
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(default_user.effective_role(), Role::User);
    }

    #[test]
    fn test_generic_message_effective_content() {
        // content field takes priority
        let with_content = GenericMessage {
            role: None,
            content: Some("from content".to_string()),
            text: Some("from text".to_string()),
            message: Some("from message".to_string()),
            sender: None,
            timestamp: None,
        };
        assert_eq!(with_content.effective_content(), "from content");

        // falls back to text
        let with_text = GenericMessage {
            role: None,
            content: None,
            text: Some("from text".to_string()),
            message: Some("from message".to_string()),
            sender: None,
            timestamp: None,
        };
        assert_eq!(with_text.effective_content(), "from text");

        // falls back to message
        let with_message = GenericMessage {
            role: None,
            content: None,
            text: None,
            message: Some("from message".to_string()),
            sender: None,
            timestamp: None,
        };
        assert_eq!(with_message.effective_content(), "from message");

        // empty when nothing
        let empty = GenericMessage {
            role: None,
            content: None,
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(empty.effective_content(), "");
    }

    // ================================================================
    // Additional migration tests
    // ================================================================

    #[test]
    fn test_import_source_serde_roundtrip() {
        let sources = [
            ImportSource::OpenClaw,
            ImportSource::ChatGPT,
            ImportSource::ClaudeExport,
            ImportSource::Generic,
        ];
        for source in &sources {
            let json = serde_json::to_string(source).unwrap();
            let deserialized: ImportSource = serde_json::from_str(&json).unwrap();
            assert_eq!(*source, deserialized);
        }
    }

    #[test]
    fn test_import_source_serde_snake_case() {
        let json = "\"open_claw\"";
        let source: ImportSource = serde_json::from_str(json).unwrap();
        assert_eq!(source, ImportSource::OpenClaw);

        // ChatGPT becomes "chat_g_p_t" with rename_all = "snake_case"
        let json = "\"chat_g_p_t\"";
        let source: ImportSource = serde_json::from_str(json).unwrap();
        assert_eq!(source, ImportSource::ChatGPT);

        let json = "\"claude_export\"";
        let source: ImportSource = serde_json::from_str(json).unwrap();
        assert_eq!(source, ImportSource::ClaudeExport);

        let json = "\"generic\"";
        let source: ImportSource = serde_json::from_str(json).unwrap();
        assert_eq!(source, ImportSource::Generic);
    }

    #[test]
    fn test_import_result_summary_single_type() {
        let sessions_only = ImportResult {
            sessions_imported: 3,
            ..Default::default()
        };
        assert_eq!(sessions_only.summary(), "Imported: 3 sessions");

        let memories_only = ImportResult {
            memories_imported: 7,
            ..Default::default()
        };
        assert_eq!(memories_only.summary(), "Imported: 7 memories");

        let skills_only = ImportResult {
            skills_imported: 1,
            ..Default::default()
        };
        assert_eq!(skills_only.summary(), "Imported: 1 skills");
    }

    #[test]
    fn test_import_result_total_imported() {
        let result = ImportResult {
            sessions_imported: 10,
            memories_imported: 20,
            skills_imported: 5,
            ..Default::default()
        };
        assert_eq!(result.total_imported(), 35);
    }

    #[test]
    fn test_import_result_has_errors_with_warnings_only() {
        let result = ImportResult {
            warnings: vec!["a warning".to_string()],
            ..Default::default()
        };
        // warnings do not count as errors
        assert!(!result.has_errors());
    }

    #[test]
    fn test_import_result_merge_preserves_source() {
        let mut a = ImportResult {
            source: Some(ImportSource::OpenClaw),
            ..Default::default()
        };
        let b = ImportResult {
            source: Some(ImportSource::Generic),
            ..Default::default()
        };
        a.merge(b);
        // source from `a` is preserved (merge doesn't overwrite source)
        assert_eq!(a.source, Some(ImportSource::OpenClaw));
    }

    #[test]
    fn test_generic_message_role_system() {
        let msg = GenericMessage {
            role: Some("system".to_string()),
            content: Some("system msg".to_string()),
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(msg.effective_role(), Role::System);
    }

    #[test]
    fn test_generic_message_role_tool() {
        let msg = GenericMessage {
            role: Some("tool".to_string()),
            content: Some("tool output".to_string()),
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(msg.effective_role(), Role::Tool);
    }

    #[test]
    fn test_generic_message_role_function() {
        let msg = GenericMessage {
            role: Some("function".to_string()),
            content: Some("function output".to_string()),
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(msg.effective_role(), Role::Tool);
    }

    #[test]
    fn test_generic_message_role_system_assistant() {
        let msg = GenericMessage {
            role: Some("system_assistant".to_string()),
            content: Some("msg".to_string()),
            text: None,
            message: None,
            sender: None,
            timestamp: None,
        };
        assert_eq!(msg.effective_role(), Role::Assistant);
    }

    #[test]
    fn test_detect_source_single_chatgpt_file() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let path = tmp.path().join("export.json");
        std::fs::write(&path, r#"[{"title": "Test", "mapping": {"n": {}}}]"#)
            .expect("Failed to write ChatGPT test file");

        let detected = MigrationEngine::detect_source(&path);
        assert_eq!(detected, ImportSource::ChatGPT);
    }

    #[test]
    fn test_detect_source_single_claude_file() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let path = tmp.path().join("export.json");
        std::fs::write(&path, r#"[{"chat_messages": []}]"#)
            .expect("Failed to write Claude export test file");

        let detected = MigrationEngine::detect_source(&path);
        assert_eq!(detected, ImportSource::ClaudeExport);
    }

    #[test]
    fn test_detect_source_single_generic_json() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let path = tmp.path().join("data.json");
        std::fs::write(&path, r#"[{"role":"user","content":"hello"}]"#)
            .expect("Failed to write generic JSON test file");

        let detected = MigrationEngine::detect_source(&path);
        assert_eq!(detected, ImportSource::Generic);
    }

    #[test]
    fn test_detect_source_openclaw_top_level_skill() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        std::fs::write(tmp.path().join("SKILL.md"), "# Skill\nTest")
            .expect("Failed to write top-level SKILL.md");

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::OpenClaw);
    }

    #[tokio::test]
    async fn test_import_generic_directory_with_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        std::fs::write(
            data_dir.join("a.json"),
            r#"[{"role":"user","content":"msg a"}]"#,
        )
        .unwrap();
        std::fs::write(
            data_dir.join("b.json"),
            r#"[{"role":"user","content":"msg b"}]"#,
        )
        .unwrap();

        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);
        let result = engine.import_generic(&data_dir).await.unwrap();

        assert_eq!(result.sessions_imported, 2);
        assert!(!result.has_errors());
    }

    #[tokio::test]
    async fn test_import_generic_nonexistent_path() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine
            .import_generic(Path::new("/nonexistent/path/abc123"))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_import_chatgpt_missing_conversations() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");
        let chatgpt_dir = tmp.path().join("chatgpt");
        std::fs::create_dir_all(&chatgpt_dir).unwrap();
        // No conversations.json

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine.import_chatgpt(&chatgpt_dir).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_import_openclaw_not_directory() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");
        let file_path = tmp.path().join("not_a_dir.txt");
        std::fs::write(&file_path, "just a file").unwrap();

        let engine = MigrationEngine::new(sessions_dir, workspace_dir);
        let result = engine.import_openclaw(&file_path).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_write_session_jsonl_creates_file() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);

        let messages = vec![Message::user("Hello"), Message::assistant("Hi!")];

        engine.write_session_jsonl(&messages).await.unwrap();

        // A session file should have been created
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(content.contains("session_start"));
        assert!(content.contains("Hello"));
        assert!(content.contains("Hi!"));
    }

    #[test]
    fn test_migration_engine_parse_messages_from_json_array() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let json = r#"[
            {"role": "user", "content": "first"},
            {"role": "assistant", "content": "second"}
        ]"#;

        let messages = engine.parse_messages_from_json(json).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages[1].content, "second");
    }

    #[test]
    fn test_migration_engine_parse_messages_from_json_with_messages_key() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let json = r#"{"messages": [
            {"role": "user", "content": "hello"}
        ]}"#;

        let messages = engine.parse_messages_from_json(json).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }

    #[test]
    fn test_migration_engine_parse_messages_from_json_with_conversation_key() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let json = r#"{"conversation": [
            {"role": "user", "content": "conv message"}
        ]}"#;

        let messages = engine.parse_messages_from_json(json).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "conv message");
    }

    #[test]
    fn test_migration_engine_parse_messages_empty_content_skipped() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let json = r#"[
            {"role": "user", "content": ""},
            {"role": "user", "content": "actual message"}
        ]"#;

        let messages = engine.parse_messages_from_json(json).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "actual message");
    }

    #[test]
    fn test_migration_engine_parse_messages_from_jsonl() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let jsonl = r#"{"role": "user", "content": "line1"}
{"role": "assistant", "content": "line2"}
"#;

        let messages = engine.parse_messages_from_jsonl(jsonl).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "line1");
        assert_eq!(messages[1].content, "line2");
    }

    #[test]
    fn test_migration_engine_parse_jsonl_skips_blank_lines() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let jsonl = r#"{"role": "user", "content": "msg"}


{"role": "assistant", "content": "reply"}
"#;

        let messages = engine.parse_messages_from_jsonl(jsonl).unwrap();
        assert_eq!(messages.len(), 2);
    }

    // ================================================================
    // New tests
    // ================================================================

    #[test]
    fn test_import_result_no_errors() {
        let result = ImportResult {
            sessions_imported: 0,
            memories_imported: 0,
            skills_imported: 0,
            errors: vec![],
            warnings: vec![],
            source: None,
        };
        assert!(!result.has_errors());
        assert_eq!(result.total_imported(), 0);
        assert_eq!(result.summary(), "Nothing imported");
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_import_result_merge_multiple() {
        let mut base = ImportResult::default();

        let r1 = ImportResult {
            sessions_imported: 1,
            memories_imported: 2,
            skills_imported: 0,
            errors: vec!["err1".to_string()],
            warnings: vec![],
            source: Some(ImportSource::ChatGPT),
        };

        let r2 = ImportResult {
            sessions_imported: 3,
            memories_imported: 0,
            skills_imported: 1,
            errors: vec![],
            warnings: vec!["warn1".to_string()],
            source: Some(ImportSource::Generic),
        };

        let r3 = ImportResult {
            sessions_imported: 0,
            memories_imported: 5,
            skills_imported: 2,
            errors: vec!["err2".to_string(), "err3".to_string()],
            warnings: vec!["warn2".to_string()],
            source: Some(ImportSource::OpenClaw),
        };

        base.merge(r1);
        base.merge(r2);
        base.merge(r3);

        assert_eq!(base.sessions_imported, 4);
        assert_eq!(base.memories_imported, 7);
        assert_eq!(base.skills_imported, 3);
        assert_eq!(base.errors.len(), 3);
        assert_eq!(base.warnings.len(), 2);
        assert_eq!(base.total_imported(), 14);
    }

    #[test]
    fn test_import_result_summary_no_imports() {
        let result = ImportResult {
            sessions_imported: 0,
            memories_imported: 0,
            skills_imported: 0,
            errors: vec!["some error".to_string()],
            warnings: vec!["a warning".to_string()],
            source: Some(ImportSource::Generic),
        };
        // Even with errors/warnings, summary shows "Nothing imported" when counts are 0
        assert_eq!(result.summary(), "Nothing imported");
    }

    #[test]
    fn test_detect_source_empty_directory() {
        let tmp = TempDir::new().unwrap();
        // Empty directory with no files
        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::Generic);
    }

    #[test]
    fn test_detect_source_with_skill_md() {
        let tmp = TempDir::new().unwrap();
        // SKILL.md at top level
        std::fs::write(tmp.path().join("SKILL.md"), "# My Skill\nDescription here.").unwrap();

        let detected = MigrationEngine::detect_source(tmp.path());
        assert_eq!(detected, ImportSource::OpenClaw);
    }

    #[test]
    fn test_generic_message_all_roles() {
        // Test all recognized role strings
        let roles_and_expected = vec![
            ("user", Role::User),
            ("assistant", Role::Assistant),
            ("bot", Role::Assistant),
            ("ai", Role::Assistant),
            ("system_assistant", Role::Assistant),
            ("system", Role::System),
            ("tool", Role::Tool),
            ("function", Role::Tool),
            ("random_string", Role::User), // Unknown defaults to User
        ];

        for (role_str, expected) in roles_and_expected {
            let msg = GenericMessage {
                role: Some(role_str.to_string()),
                content: Some("test".to_string()),
                text: None,
                message: None,
                sender: None,
                timestamp: None,
            };
            assert_eq!(
                msg.effective_role(),
                expected,
                "Role '{}' should map to {:?}",
                role_str,
                expected
            );
        }
    }

    #[tokio::test]
    async fn test_import_generic_single_file() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        let single_file = tmp.path().join("single.json");
        std::fs::write(
            &single_file,
            r#"[{"role": "user", "content": "only message"}]"#,
        )
        .unwrap();

        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);
        let result = engine.import_generic(&single_file).await.unwrap();

        assert_eq!(result.sessions_imported, 1);
        assert!(!result.has_errors());

        // Verify session file exists
        let files: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(files.len(), 1);
    }

    #[tokio::test]
    async fn test_import_generic_deeply_nested() {
        let tmp = TempDir::new().unwrap();
        let sessions_dir = tmp.path().join("sessions");
        let workspace_dir = tmp.path().join("workspace");

        // Create a deeply nested dir with a JSON file
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("deep.json"),
            r#"[{"role": "user", "content": "deep msg"}]"#,
        )
        .unwrap();

        // import_generic only looks at the immediate directory, not recursively
        // So importing the nested dir should find the file
        let engine = MigrationEngine::new(sessions_dir.clone(), workspace_dir);
        let result = engine.import_generic(&nested).await.unwrap();

        assert_eq!(result.sessions_imported, 1);
    }

    #[test]
    fn test_migration_engine_parse_empty_json() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        let json = "[]";
        let messages = engine.parse_messages_from_json(json).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn test_migration_engine_parse_malformed_jsonl() {
        let tmp = TempDir::new().unwrap();
        let engine =
            MigrationEngine::new(tmp.path().join("sessions"), tmp.path().join("workspace"));

        // Malformed JSONL: some lines are not valid JSON
        let jsonl = r#"{"role": "user", "content": "valid line"}
this is not json at all
{broken json{{{
{"role": "assistant", "content": "another valid line"}
"#;

        // parse_messages_from_jsonl should skip invalid lines gracefully
        let messages = engine.parse_messages_from_jsonl(jsonl).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "valid line");
        assert_eq!(messages[1].content, "another valid line");
    }
}
