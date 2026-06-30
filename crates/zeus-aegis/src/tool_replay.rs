//! Tool Replay — record and replay tool execution sequences.
//!
//! Captures tool invocations with their arguments and results for:
//! - **Audit trails**: Full reproducible history of what tools did
//! - **Regression testing**: Replay recorded sequences to detect behavior changes
//! - **Security review**: Inspect tool chains for privilege escalation patterns
//! - **Debugging**: Step through tool execution history with diffs

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the tool replay system.
#[derive(Debug, Clone)]
pub struct ReplayConfig {
    /// Maximum number of entries to retain in the recording buffer.
    pub max_entries: usize,
    /// Whether to record tool arguments (may contain sensitive data).
    pub record_arguments: bool,
    /// Whether to record tool results.
    pub record_results: bool,
    /// Maximum size of a single argument/result field (bytes, truncated beyond).
    pub max_field_size: usize,
    /// Tags to auto-apply to all recordings.
    pub default_tags: Vec<String>,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            record_arguments: true,
            record_results: true,
            max_field_size: 4096,
            default_tags: Vec::new(),
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// A recorded tool invocation.
#[derive(Debug, Clone)]
pub struct ReplayEntry {
    /// Unique sequence number within this recorder.
    pub seq: u64,
    /// Session ID this invocation belongs to.
    pub session_id: String,
    /// Tool name that was invoked.
    pub tool_name: String,
    /// Tool arguments (JSON string, may be truncated).
    pub arguments: Option<String>,
    /// Tool result (may be truncated).
    pub result: Option<String>,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
    /// Unix timestamp of invocation.
    pub timestamp: u64,
    /// User/agent who invoked the tool.
    pub actor: String,
    /// Tags for categorization and filtering.
    pub tags: Vec<String>,
}

/// Outcome of comparing two replay entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffResult {
    /// Entries match (same tool, same result success status).
    Match,
    /// Tool name differs.
    ToolMismatch { expected: String, actual: String },
    /// Success status differs.
    OutcomeMismatch { expected: bool, actual: bool },
    /// Result content differs.
    ResultDiff {
        expected: Option<String>,
        actual: Option<String>,
    },
    /// Entry missing in replay.
    Missing { expected_seq: u64 },
    /// Extra entry in replay not in recording.
    Extra { actual_seq: u64 },
}

/// A recorded sequence that can be replayed.
#[derive(Debug, Clone)]
pub struct Recording {
    /// Unique recording identifier.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Session this was recorded from.
    pub session_id: String,
    /// The recorded entries in order.
    pub entries: Vec<ReplayEntry>,
    /// When the recording was created.
    pub created_at: u64,
    /// Tags on the recording.
    pub tags: Vec<String>,
}

/// Result of replaying a recording against new execution.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    /// Recording that was replayed.
    pub recording_id: String,
    /// Per-entry diff results.
    pub diffs: Vec<DiffResult>,
    /// Number of matching entries.
    pub matches: usize,
    /// Number of mismatches.
    pub mismatches: usize,
    /// Whether the replay fully matches.
    pub fully_matched: bool,
}

/// Statistics about the replay system.
#[derive(Debug, Clone, Default)]
pub struct ReplayStats {
    /// Total entries recorded.
    pub entries_recorded: u64,
    /// Total recordings saved.
    pub recordings_saved: usize,
    /// Total replays performed.
    pub replays_performed: usize,
    /// Total entries pruned.
    pub entries_pruned: u64,
}

/// Filter criteria for querying recorded entries.
#[derive(Debug, Clone, Default)]
pub struct ReplayFilter {
    /// Filter by session ID.
    pub session_id: Option<String>,
    /// Filter by tool name.
    pub tool_name: Option<String>,
    /// Filter by actor.
    pub actor: Option<String>,
    /// Filter by success status.
    pub success: Option<bool>,
    /// Filter by tag (any match).
    pub tag: Option<String>,
    /// Only entries after this timestamp.
    pub after: Option<u64>,
    /// Only entries before this timestamp.
    pub before: Option<u64>,
    /// Maximum results to return.
    pub limit: Option<usize>,
}

// ============================================================================
// Replay Recorder
// ============================================================================

/// The tool replay recorder and comparator.
pub struct ToolReplayRecorder {
    config: ReplayConfig,
    /// Live recording buffer.
    entries: Vec<ReplayEntry>,
    /// Saved recordings for replay comparison.
    recordings: Vec<Recording>,
    /// Next sequence number.
    next_seq: u64,
    /// Statistics.
    stats: ReplayStats,
}

impl ToolReplayRecorder {
    /// Create with default configuration.
    pub fn new() -> Self {
        Self {
            config: ReplayConfig::default(),
            entries: Vec::new(),
            recordings: Vec::new(),
            next_seq: 1,
            stats: ReplayStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: ReplayConfig) -> Self {
        Self {
            config,
            entries: Vec::new(),
            recordings: Vec::new(),
            next_seq: 1,
            stats: ReplayStats::default(),
        }
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: ReplayConfig) {
        self.config = config;
    }

    /// Get current statistics.
    pub fn stats(&self) -> &ReplayStats {
        &self.stats
    }

    /// Record a tool invocation.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        session_id: &str,
        tool_name: &str,
        arguments: Option<&str>,
        result: Option<&str>,
        success: bool,
        duration_ms: u64,
        actor: &str,
        tags: Vec<String>,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let truncate = |s: &str| -> String {
            if s.len() > self.config.max_field_size {
                format!("{}...[truncated]", &s[..zeus_core::floor_char_boundary(s, self.config.max_field_size)])
            } else {
                s.to_string()
            }
        };

        let arguments = if self.config.record_arguments {
            arguments.map(&truncate)
        } else {
            None
        };

        let result = if self.config.record_results {
            result.map(truncate)
        } else {
            None
        };

        let mut all_tags = self.config.default_tags.clone();
        all_tags.extend(tags);

        let entry = ReplayEntry {
            seq,
            session_id: session_id.to_string(),
            tool_name: tool_name.to_string(),
            arguments,
            result,
            success,
            duration_ms,
            timestamp: now,
            actor: actor.to_string(),
            tags: all_tags,
        };

        self.entries.push(entry);
        self.stats.entries_recorded += 1;

        // Prune if over limit
        if self.entries.len() > self.config.max_entries {
            let excess = self.entries.len() - self.config.max_entries;
            self.entries.drain(..excess);
            self.stats.entries_pruned += excess as u64;
        }

        seq
    }

    /// Get all recorded entries.
    pub fn entries(&self) -> &[ReplayEntry] {
        &self.entries
    }

    /// Query entries with a filter.
    pub fn query(&self, filter: &ReplayFilter) -> Vec<&ReplayEntry> {
        let mut results: Vec<&ReplayEntry> = self
            .entries
            .iter()
            .filter(|e| {
                if let Some(ref sid) = filter.session_id
                    && e.session_id != *sid
                {
                    return false;
                }
                if let Some(ref tn) = filter.tool_name
                    && e.tool_name != *tn
                {
                    return false;
                }
                if let Some(ref actor) = filter.actor
                    && e.actor != *actor
                {
                    return false;
                }
                if let Some(success) = filter.success
                    && e.success != success
                {
                    return false;
                }
                if let Some(ref tag) = filter.tag
                    && !e.tags.contains(tag)
                {
                    return false;
                }
                if let Some(after) = filter.after
                    && e.timestamp < after
                {
                    return false;
                }
                if let Some(before) = filter.before
                    && e.timestamp > before
                {
                    return false;
                }
                true
            })
            .collect();

        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }

        results
    }

    /// Save current entries for a session as a named recording.
    pub fn save_recording(
        &mut self,
        session_id: &str,
        label: &str,
        tags: Vec<String>,
    ) -> Option<String> {
        let session_entries: Vec<ReplayEntry> = self
            .entries
            .iter()
            .filter(|e| e.session_id == session_id)
            .cloned()
            .collect();

        if session_entries.is_empty() {
            return None;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let id = format!("rec-{}-{}", session_id, now);

        let recording = Recording {
            id: id.clone(),
            label: label.to_string(),
            session_id: session_id.to_string(),
            entries: session_entries,
            created_at: now,
            tags,
        };

        self.recordings.push(recording);
        self.stats.recordings_saved += 1;

        Some(id)
    }

    /// Get all saved recordings.
    pub fn recordings(&self) -> &[Recording] {
        &self.recordings
    }

    /// Get a specific recording by ID.
    pub fn get_recording(&self, id: &str) -> Option<&Recording> {
        self.recordings.iter().find(|r| r.id == id)
    }

    /// Delete a recording by ID.
    pub fn delete_recording(&mut self, id: &str) -> bool {
        let before = self.recordings.len();
        self.recordings.retain(|r| r.id != id);
        self.recordings.len() < before
    }

    /// Compare a recording against a set of new entries (replay comparison).
    pub fn compare(
        &mut self,
        recording_id: &str,
        actual_entries: &[ReplayEntry],
    ) -> Option<ReplayReport> {
        let recording = self.recordings.iter().find(|r| r.id == recording_id)?;
        let expected = &recording.entries;

        let mut diffs = Vec::new();
        let max_len = expected.len().max(actual_entries.len());

        for i in 0..max_len {
            match (expected.get(i), actual_entries.get(i)) {
                (Some(exp), Some(act)) => {
                    if exp.tool_name != act.tool_name {
                        diffs.push(DiffResult::ToolMismatch {
                            expected: exp.tool_name.clone(),
                            actual: act.tool_name.clone(),
                        });
                    } else if exp.success != act.success {
                        diffs.push(DiffResult::OutcomeMismatch {
                            expected: exp.success,
                            actual: act.success,
                        });
                    } else if exp.result != act.result {
                        diffs.push(DiffResult::ResultDiff {
                            expected: exp.result.clone(),
                            actual: act.result.clone(),
                        });
                    } else {
                        diffs.push(DiffResult::Match);
                    }
                }
                (Some(exp), None) => {
                    diffs.push(DiffResult::Missing {
                        expected_seq: exp.seq,
                    });
                }
                (None, Some(act)) => {
                    diffs.push(DiffResult::Extra {
                        actual_seq: act.seq,
                    });
                }
                (None, None) => unreachable!(),
            }
        }

        let matches = diffs
            .iter()
            .filter(|d| matches!(d, DiffResult::Match))
            .count();
        let mismatches = diffs.len() - matches;
        let fully_matched = mismatches == 0;

        self.stats.replays_performed += 1;

        Some(ReplayReport {
            recording_id: recording_id.to_string(),
            diffs,
            matches,
            mismatches,
            fully_matched,
        })
    }

    /// Get tool invocation frequency counts.
    pub fn tool_frequency(&self) -> Vec<(String, usize)> {
        let mut freq: HashMap<String, usize> = HashMap::new();
        for e in &self.entries {
            *freq.entry(e.tool_name.clone()).or_insert(0) += 1;
        }
        let mut sorted: Vec<(String, usize)> = freq.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted
    }

    /// Detect tool chains: sequences of tools invoked within a session.
    pub fn detect_chains(&self, session_id: &str, min_length: usize) -> Vec<Vec<String>> {
        let session_entries: Vec<&ReplayEntry> = self
            .entries
            .iter()
            .filter(|e| e.session_id == session_id)
            .collect();

        if session_entries.len() < min_length {
            return Vec::new();
        }

        // Extract tool name sequence
        let tool_seq: Vec<String> = session_entries
            .iter()
            .map(|e| e.tool_name.clone())
            .collect();

        // Find repeating subsequences of at least min_length
        let mut chains: Vec<Vec<String>> = Vec::new();

        for window_size in min_length..=tool_seq.len().min(10) {
            let mut seen: HashMap<Vec<String>, usize> = HashMap::new();
            for window in tool_seq.windows(window_size) {
                let key: Vec<String> = window.to_vec();
                *seen.entry(key).or_insert(0) += 1;
            }
            for (chain, count) in seen {
                if count >= 2 {
                    chains.push(chain);
                }
            }
        }

        // Deduplicate — remove chains that are subsets of longer chains
        chains.sort_by_key(|c| std::cmp::Reverse(c.len()));
        let mut unique_chains: Vec<Vec<String>> = Vec::new();
        for chain in &chains {
            let chain_str = chain.join(",");
            let is_subset = unique_chains.iter().any(|longer| {
                let longer_str = longer.join(",");
                longer_str.contains(&chain_str) && longer.len() > chain.len()
            });
            if !is_subset {
                unique_chains.push(chain.clone());
            }
        }

        unique_chains
    }

    /// Clear all recorded entries (but keep saved recordings).
    pub fn clear_entries(&mut self) {
        self.entries.clear();
    }
}

impl Default for ToolReplayRecorder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn record_sample(recorder: &mut ToolReplayRecorder) {
        recorder.record(
            "sess-1",
            "read_file",
            Some(r#"{"path":"/tmp/foo"}"#),
            Some("file contents"),
            true,
            15,
            "user-1",
            vec![],
        );
        recorder.record(
            "sess-1",
            "edit_file",
            Some(r#"{"path":"/tmp/foo","content":"new"}"#),
            Some("ok"),
            true,
            25,
            "user-1",
            vec!["write".into()],
        );
        recorder.record(
            "sess-1",
            "shell",
            Some(r#"{"cmd":"ls"}"#),
            Some("dir listing"),
            true,
            100,
            "user-1",
            vec!["exec".into()],
        );
        recorder.record(
            "sess-2",
            "web_fetch",
            Some(r#"{"url":"http://example.com"}"#),
            None,
            false,
            5000,
            "user-2",
            vec!["network".into()],
        );
    }

    #[test]
    fn test_default_config() {
        let config = ReplayConfig::default();
        assert_eq!(config.max_entries, 10_000);
        assert!(config.record_arguments);
        assert!(config.record_results);
        assert_eq!(config.max_field_size, 4096);
    }

    #[test]
    fn test_new_recorder() {
        let r = ToolReplayRecorder::new();
        assert!(r.entries().is_empty());
        assert!(r.recordings().is_empty());
        assert_eq!(r.stats().entries_recorded, 0);
    }

    #[test]
    fn test_record_basic() {
        let mut r = ToolReplayRecorder::new();
        let seq = r.record(
            "sess-1",
            "read_file",
            Some("args"),
            Some("result"),
            true,
            10,
            "actor",
            vec![],
        );
        assert_eq!(seq, 1);
        assert_eq!(r.entries().len(), 1);
        assert_eq!(r.entries()[0].tool_name, "read_file");
        assert_eq!(r.entries()[0].session_id, "sess-1");
        assert!(r.entries()[0].success);
        assert_eq!(r.stats().entries_recorded, 1);
    }

    #[test]
    fn test_record_sequential_seq() {
        let mut r = ToolReplayRecorder::new();
        let s1 = r.record("s", "t1", None, None, true, 0, "a", vec![]);
        let s2 = r.record("s", "t2", None, None, true, 0, "a", vec![]);
        let s3 = r.record("s", "t3", None, None, true, 0, "a", vec![]);
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn test_record_without_arguments() {
        let mut r = ToolReplayRecorder::with_config(ReplayConfig {
            record_arguments: false,
            ..ReplayConfig::default()
        });
        r.record(
            "s",
            "tool",
            Some("secret_args"),
            Some("result"),
            true,
            0,
            "a",
            vec![],
        );
        assert!(r.entries()[0].arguments.is_none());
        assert!(r.entries()[0].result.is_some());
    }

    #[test]
    fn test_record_without_results() {
        let mut r = ToolReplayRecorder::with_config(ReplayConfig {
            record_results: false,
            ..ReplayConfig::default()
        });
        r.record(
            "s",
            "tool",
            Some("args"),
            Some("secret_result"),
            true,
            0,
            "a",
            vec![],
        );
        assert!(r.entries()[0].arguments.is_some());
        assert!(r.entries()[0].result.is_none());
    }

    #[test]
    fn test_field_truncation() {
        let mut r = ToolReplayRecorder::with_config(ReplayConfig {
            max_field_size: 10,
            ..ReplayConfig::default()
        });
        let long_arg = "a".repeat(100);
        r.record("s", "tool", Some(&long_arg), None, true, 0, "a", vec![]);
        let recorded = r.entries()[0].arguments.as_ref().unwrap();
        assert!(recorded.len() < 100);
        assert!(recorded.contains("[truncated]"));
    }

    #[test]
    fn test_default_tags_applied() {
        let mut r = ToolReplayRecorder::with_config(ReplayConfig {
            default_tags: vec!["audit".into()],
            ..ReplayConfig::default()
        });
        r.record("s", "tool", None, None, true, 0, "a", vec!["extra".into()]);
        assert!(r.entries()[0].tags.contains(&"audit".to_string()));
        assert!(r.entries()[0].tags.contains(&"extra".to_string()));
    }

    #[test]
    fn test_max_entries_pruning() {
        let mut r = ToolReplayRecorder::with_config(ReplayConfig {
            max_entries: 3,
            ..ReplayConfig::default()
        });
        for i in 0..5 {
            r.record(
                "s",
                &format!("tool-{}", i),
                None,
                None,
                true,
                0,
                "a",
                vec![],
            );
        }
        assert_eq!(r.entries().len(), 3);
        // Should keep the latest 3
        assert_eq!(r.entries()[0].tool_name, "tool-2");
        assert_eq!(r.entries()[2].tool_name, "tool-4");
        assert_eq!(r.stats().entries_pruned, 2);
    }

    #[test]
    fn test_query_by_session() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            session_id: Some("sess-1".into()),
            ..Default::default()
        });
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_query_by_tool_name() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            tool_name: Some("shell".into()),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name, "shell");
    }

    #[test]
    fn test_query_by_success() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            success: Some(false),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name, "web_fetch");
    }

    #[test]
    fn test_query_by_tag() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            tag: Some("write".into()),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_name, "edit_file");
    }

    #[test]
    fn test_query_with_limit() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            limit: Some(2),
            ..Default::default()
        });
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_query_combined_filters() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let results = r.query(&ReplayFilter {
            session_id: Some("sess-1".into()),
            success: Some(true),
            ..Default::default()
        });
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_save_recording() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let id = r.save_recording("sess-1", "test run", vec!["ci".into()]);
        assert!(id.is_some());
        let id = id.unwrap();
        assert!(id.starts_with("rec-sess-1-"));

        assert_eq!(r.recordings().len(), 1);
        let rec = r.get_recording(&id).unwrap();
        assert_eq!(rec.entries.len(), 3); // Only sess-1 entries
        assert_eq!(rec.label, "test run");
        assert!(rec.tags.contains(&"ci".to_string()));
    }

    #[test]
    fn test_save_recording_empty_session() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let id = r.save_recording("nonexistent", "empty", vec![]);
        assert!(id.is_none());
    }

    #[test]
    fn test_delete_recording() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let id = r.save_recording("sess-1", "test", vec![]).unwrap();
        assert!(r.delete_recording(&id));
        assert!(r.recordings().is_empty());
        assert!(!r.delete_recording(&id)); // Already deleted
    }

    #[test]
    fn test_compare_matching() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        let id = r.save_recording("sess-1", "baseline", vec![]).unwrap();

        // Replay with same entries
        let actual: Vec<ReplayEntry> = r
            .entries()
            .iter()
            .filter(|e| e.session_id == "sess-1")
            .cloned()
            .collect();

        let report = r.compare(&id, &actual).unwrap();
        assert!(report.fully_matched);
        assert_eq!(report.matches, 3);
        assert_eq!(report.mismatches, 0);
    }

    #[test]
    fn test_compare_tool_mismatch() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        let id = r.save_recording("s", "baseline", vec![]).unwrap();

        let actual = vec![ReplayEntry {
            seq: 1,
            session_id: "s".into(),
            tool_name: "write_file".into(),
            arguments: None,
            result: None,
            success: true,
            duration_ms: 0,
            timestamp: 0,
            actor: "a".into(),
            tags: vec![],
        }];

        let report = r.compare(&id, &actual).unwrap();
        assert!(!report.fully_matched);
        assert!(matches!(report.diffs[0], DiffResult::ToolMismatch { .. }));
    }

    #[test]
    fn test_compare_outcome_mismatch() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "shell", None, None, true, 0, "a", vec![]);
        let id = r.save_recording("s", "baseline", vec![]).unwrap();

        let actual = vec![ReplayEntry {
            seq: 1,
            session_id: "s".into(),
            tool_name: "shell".into(),
            arguments: None,
            result: None,
            success: false,
            duration_ms: 0,
            timestamp: 0,
            actor: "a".into(),
            tags: vec![],
        }];

        let report = r.compare(&id, &actual).unwrap();
        assert!(!report.fully_matched);
        assert!(matches!(
            report.diffs[0],
            DiffResult::OutcomeMismatch { .. }
        ));
    }

    #[test]
    fn test_compare_missing_entries() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "t1", None, None, true, 0, "a", vec![]);
        r.record("s", "t2", None, None, true, 0, "a", vec![]);
        let id = r.save_recording("s", "baseline", vec![]).unwrap();

        // Replay with only one entry
        let actual = vec![ReplayEntry {
            seq: 1,
            session_id: "s".into(),
            tool_name: "t1".into(),
            arguments: None,
            result: None,
            success: true,
            duration_ms: 0,
            timestamp: 0,
            actor: "a".into(),
            tags: vec![],
        }];

        let report = r.compare(&id, &actual).unwrap();
        assert!(!report.fully_matched);
        assert_eq!(report.mismatches, 1);
        assert!(matches!(report.diffs[1], DiffResult::Missing { .. }));
    }

    #[test]
    fn test_compare_extra_entries() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "t1", None, None, true, 0, "a", vec![]);
        let id = r.save_recording("s", "baseline", vec![]).unwrap();

        let actual = vec![
            ReplayEntry {
                seq: 1,
                session_id: "s".into(),
                tool_name: "t1".into(),
                arguments: None,
                result: None,
                success: true,
                duration_ms: 0,
                timestamp: 0,
                actor: "a".into(),
                tags: vec![],
            },
            ReplayEntry {
                seq: 2,
                session_id: "s".into(),
                tool_name: "t2".into(),
                arguments: None,
                result: None,
                success: true,
                duration_ms: 0,
                timestamp: 0,
                actor: "a".into(),
                tags: vec![],
            },
        ];

        let report = r.compare(&id, &actual).unwrap();
        assert!(!report.fully_matched);
        assert!(matches!(report.diffs[1], DiffResult::Extra { .. }));
    }

    #[test]
    fn test_compare_nonexistent_recording() {
        let mut r = ToolReplayRecorder::new();
        let report = r.compare("nonexistent", &[]);
        assert!(report.is_none());
    }

    #[test]
    fn test_tool_frequency() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        r.record("s", "shell", None, None, true, 0, "a", vec![]);
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);

        let freq = r.tool_frequency();
        assert_eq!(freq[0], ("read_file".to_string(), 3));
        assert_eq!(freq[1], ("shell".to_string(), 1));
    }

    #[test]
    fn test_detect_chains() {
        let mut r = ToolReplayRecorder::new();
        // Pattern: read → edit → shell (repeated twice)
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        r.record("s", "edit_file", None, None, true, 0, "a", vec![]);
        r.record("s", "shell", None, None, true, 0, "a", vec![]);
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        r.record("s", "edit_file", None, None, true, 0, "a", vec![]);
        r.record("s", "shell", None, None, true, 0, "a", vec![]);

        let chains = r.detect_chains("s", 2);
        assert!(!chains.is_empty());
        // Should detect the read→edit→shell pattern
        let has_pattern = chains.iter().any(|c| {
            c.len() >= 2
                && c.contains(&"read_file".to_string())
                && c.contains(&"edit_file".to_string())
        });
        assert!(has_pattern);
    }

    #[test]
    fn test_detect_chains_too_few_entries() {
        let mut r = ToolReplayRecorder::new();
        r.record("s", "read_file", None, None, true, 0, "a", vec![]);
        let chains = r.detect_chains("s", 3);
        assert!(chains.is_empty());
    }

    #[test]
    fn test_clear_entries() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        assert!(!r.entries().is_empty());
        r.clear_entries();
        assert!(r.entries().is_empty());
    }

    #[test]
    fn test_clear_preserves_recordings() {
        let mut r = ToolReplayRecorder::new();
        record_sample(&mut r);
        r.save_recording("sess-1", "saved", vec![]);
        r.clear_entries();
        assert!(r.entries().is_empty());
        assert_eq!(r.recordings().len(), 1);
    }

    #[test]
    fn test_set_config() {
        let mut r = ToolReplayRecorder::new();
        assert_eq!(r.config.max_entries, 10_000);
        r.set_config(ReplayConfig {
            max_entries: 100,
            ..ReplayConfig::default()
        });
        assert_eq!(r.config.max_entries, 100);
    }
}
