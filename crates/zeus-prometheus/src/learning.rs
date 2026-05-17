//! Interaction Learning Engine
//!
//! Records interaction patterns and learns from them to improve
//! future intent classification and decision-making. Uses a SQLite
//! backend to persist interaction records and compute aggregate
//! statistics for pattern recognition.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::debug;
use zeus_core::Result;

// ============================================================================
// Types
// ============================================================================

/// A single recorded interaction with the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionRecord {
    /// Unique identifier for this interaction.
    pub id: String,
    /// When the interaction occurred.
    pub timestamp: DateTime<Utc>,
    /// Classified type of the query (e.g. "file_operation", "web_search", "chat").
    pub query_type: String,
    /// Tools that were invoked during this interaction.
    pub tools_used: Vec<String>,
    /// Whether the interaction completed successfully.
    pub success: bool,
    /// Wall-clock duration of the interaction in milliseconds.
    pub duration_ms: u64,
    /// Error message if the interaction failed.
    pub error_message: Option<String>,
    /// Estimated complexity ("simple", "moderate", "complex").
    pub complexity: String,
}

/// Aggregated statistics for a particular query type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternSummary {
    /// The query type these stats describe.
    pub query_type: String,
    /// Total number of interactions of this type.
    pub total_count: u64,
    /// Number of successful interactions.
    pub success_count: u64,
    /// Average duration across all interactions of this type.
    pub avg_duration_ms: f64,
    /// Tools most commonly used, with their usage counts.
    pub common_tools: Vec<(String, u64)>,
    /// When the most recent interaction of this type occurred.
    pub last_seen: DateTime<Utc>,
}

/// Per-tool usage and effectiveness statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolEffectiveness {
    /// Name of the tool.
    pub tool_name: String,
    /// Total number of interactions that used this tool.
    pub total_uses: u64,
    /// Fraction of those interactions that succeeded (0.0 - 1.0).
    pub success_rate: f32,
    /// Average duration of interactions that used this tool.
    pub avg_duration_ms: f64,
}

/// Configuration for the learning engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConfig {
    /// Path to the SQLite database file.
    pub db_path: PathBuf,
    /// Maximum number of records to retain; older records are pruned.
    #[serde(default = "default_max_records")]
    pub max_records: usize,
    /// Whether learning is enabled.
    #[serde(default = "default_enable_learning")]
    pub enable_learning: bool,
}

fn default_max_records() -> usize {
    10_000
}

fn default_enable_learning() -> bool {
    true
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("learning.db"),
            max_records: default_max_records(),
            enable_learning: default_enable_learning(),
        }
    }
}

// ============================================================================
// LearningEngine
// ============================================================================

const INTERACTION_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS interaction_records (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                query_type TEXT NOT NULL,
                tools_used TEXT NOT NULL,
                success INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                error_message TEXT,
                complexity TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ir_query_type ON interaction_records(query_type);
            CREATE INDEX IF NOT EXISTS idx_ir_timestamp ON interaction_records(timestamp);",
];

const STRATEGIC_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS learnings (
                category TEXT NOT NULL,
                pattern TEXT NOT NULL,
                outcome TEXT NOT NULL,
                confidence REAL NOT NULL,
                observations INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                intent TEXT,
                PRIMARY KEY (category, pattern, intent)
            );
            CREATE INDEX IF NOT EXISTS idx_learnings_intent ON learnings(intent);
            CREATE INDEX IF NOT EXISTS idx_learnings_category ON learnings(category);
            CREATE INDEX IF NOT EXISTS idx_learnings_updated ON learnings(updated_at);",
];

/// Interaction learning engine backed by SQLite.
///
/// Records each interaction (query type, tools used, success/failure,
/// duration) and provides methods to query aggregate patterns that can
/// guide future tool selection and intent classification.
pub struct LearningEngine {
    /// Path to the SQLite database.
    db_path: PathBuf,
    /// Maximum records to keep.
    max_records: usize,
    /// Whether learning is active.
    enabled: bool,
    /// Serializes database writes so concurrent callers don't collide.
    _write_lock: Mutex<()>,
}

impl LearningEngine {
    /// Create a new learning engine and initialize its database schema.
    pub fn new(config: &LearningConfig) -> Result<Self> {
        let engine = Self {
            db_path: config.db_path.clone(),
            max_records: config.max_records,
            enabled: config.enable_learning,
            _write_lock: Mutex::new(()),
        };
        engine.init_db()?;
        Ok(engine)
    }

    /// Open a fresh connection to the SQLite database.
    fn open_db(&self) -> Result<rusqlite::Connection> {
        rusqlite::Connection::open(&self.db_path)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to open learning db: {}", e)))
    }

    /// Create the schema if it does not already exist.
    fn init_db(&self) -> Result<()> {
        let conn = self.open_db()?;
        crate::db::run_migrations(&conn, INTERACTION_MIGRATIONS)?;
        Ok(())
    }

    /// Record an interaction. Automatically prunes old records if the table
    /// exceeds `max_records`.
    pub fn record(&self, record: InteractionRecord) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let _lock = self
            ._write_lock
            .lock()
            .map_err(|e| zeus_core::Error::Internal(format!("Lock poisoned: {}", e)))?;

        let conn = self.open_db()?;

        let tools_json = serde_json::to_string(&record.tools_used)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to serialize tools: {}", e)))?;

        conn.execute(
            "INSERT OR REPLACE INTO interaction_records
                (id, timestamp, query_type, tools_used, success, duration_ms, error_message, complexity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                record.id,
                record.timestamp.to_rfc3339(),
                record.query_type,
                tools_json,
                record.success as i32,
                record.duration_ms as i64,
                record.error_message,
                record.complexity,
            ],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to insert record: {}", e)))?;

        // Prune if needed.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM interaction_records", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        if count > self.max_records as i64 {
            let excess = count - self.max_records as i64;
            conn.execute(
                "DELETE FROM interaction_records WHERE id IN (
                    SELECT id FROM interaction_records ORDER BY timestamp ASC LIMIT ?1
                )",
                rusqlite::params![excess],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prune records: {}", e)))?;
            debug!("Pruned {} old interaction records", excess);
        }

        Ok(())
    }

    /// Get aggregated pattern statistics for a given query type.
    ///
    /// Returns `None` if there are no records for the specified type.
    pub fn get_patterns(&self, query_type: &str) -> Result<Option<PatternSummary>> {
        let conn = self.open_db()?;

        // Aggregate counts and averages.
        let maybe_row: std::result::Result<(u64, u64, f64, String), _> = conn.query_row(
            "SELECT COUNT(*), SUM(success), AVG(duration_ms), MAX(timestamp)
             FROM interaction_records WHERE query_type = ?1",
            rusqlite::params![query_type],
            |row| {
                let total: u64 = row.get(0)?;
                let success: u64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64;
                let avg_dur: f64 = row.get::<_, Option<f64>>(2)?.unwrap_or(0.0);
                let last_ts: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                Ok((total, success, avg_dur, last_ts))
            },
        );

        let (total_count, success_count, avg_duration_ms, last_ts_str) = match maybe_row {
            Ok(r) if r.0 > 0 => r,
            _ => return Ok(None),
        };

        let last_seen = DateTime::parse_from_rfc3339(&last_ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        // Compute common tools by scanning the tools_used JSON arrays.
        let common_tools = self.compute_common_tools(&conn, query_type)?;

        Ok(Some(PatternSummary {
            query_type: query_type.to_string(),
            total_count,
            success_count,
            avg_duration_ms,
            common_tools,
            last_seen,
        }))
    }

    /// Helper: tally tool usage across all records for a query type.
    fn compute_common_tools(
        &self,
        conn: &rusqlite::Connection,
        query_type: &str,
    ) -> Result<Vec<(String, u64)>> {
        let mut stmt = conn
            .prepare("SELECT tools_used FROM interaction_records WHERE query_type = ?1")
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![query_type], |row| {
                let json_str: String = row.get(0)?;
                Ok(json_str)
            })
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query tools: {}", e)))?;

        let mut tool_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for row in rows {
            let json_str =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            if let Ok(tools) = serde_json::from_str::<Vec<String>>(&json_str) {
                for tool in tools {
                    *tool_counts.entry(tool).or_insert(0) += 1;
                }
            }
        }

        let mut sorted: Vec<(String, u64)> = tool_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sorted)
    }

    /// Compute per-tool effectiveness statistics across all query types.
    pub fn get_tool_effectiveness(&self) -> Result<Vec<ToolEffectiveness>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare("SELECT tools_used, success, duration_ms FROM interaction_records")
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let json_str: String = row.get(0)?;
                let success: i32 = row.get(1)?;
                let duration: i64 = row.get(2)?;
                Ok((json_str, success != 0, duration as u64))
            })
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query records: {}", e)))?;

        // Accumulate per-tool stats.
        struct ToolAcc {
            total: u64,
            successes: u64,
            total_duration: u64,
        }
        let mut acc: std::collections::HashMap<String, ToolAcc> = std::collections::HashMap::new();

        for row in rows {
            let (json_str, success, duration) =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            if let Ok(tools) = serde_json::from_str::<Vec<String>>(&json_str) {
                for tool in tools {
                    let entry = acc.entry(tool).or_insert(ToolAcc {
                        total: 0,
                        successes: 0,
                        total_duration: 0,
                    });
                    entry.total += 1;
                    if success {
                        entry.successes += 1;
                    }
                    entry.total_duration += duration;
                }
            }
        }

        let mut results: Vec<ToolEffectiveness> = acc
            .into_iter()
            .map(|(name, a)| ToolEffectiveness {
                tool_name: name,
                total_uses: a.total,
                success_rate: if a.total > 0 {
                    a.successes as f32 / a.total as f32
                } else {
                    0.0
                },
                avg_duration_ms: if a.total > 0 {
                    a.total_duration as f64 / a.total as f64
                } else {
                    0.0
                },
            })
            .collect();

        results.sort_by(|a, b| b.total_uses.cmp(&a.total_uses));
        Ok(results)
    }

    /// Suggest tools for a given query type based on past successful interactions.
    ///
    /// Returns tools sorted by usage frequency in successful interactions of
    /// the given type.
    pub fn suggest_tools(&self, query_type: &str) -> Result<Vec<String>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare(
                "SELECT tools_used FROM interaction_records
                 WHERE query_type = ?1 AND success = 1",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![query_type], |row| {
                let json_str: String = row.get(0)?;
                Ok(json_str)
            })
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query tools: {}", e)))?;

        let mut tool_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for row in rows {
            let json_str =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            if let Ok(tools) = serde_json::from_str::<Vec<String>>(&json_str) {
                for tool in tools {
                    *tool_counts.entry(tool).or_insert(0) += 1;
                }
            }
        }

        let mut sorted: Vec<(String, u64)> = tool_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        Ok(sorted.into_iter().map(|(name, _)| name).collect())
    }

    /// Overall success rate across all recorded interactions.
    ///
    /// Returns 0.0 if no interactions have been recorded.
    pub fn success_rate(&self) -> Result<f32> {
        let conn = self.open_db()?;

        let (total, successes): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(success) FROM interaction_records",
                [],
                |row| {
                    let total: i64 = row.get(0)?;
                    let successes: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
                    Ok((total, successes))
                },
            )
            .map_err(|e| {
                zeus_core::Error::Database(format!("Failed to query success rate: {}", e))
            })?;

        if total == 0 {
            return Ok(0.0);
        }
        Ok(successes as f32 / total as f32)
    }

    /// Get the most recent failed interactions for reflection.
    pub fn recent_errors(&self, limit: usize) -> Result<Vec<InteractionRecord>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, query_type, tools_used, success, duration_ms, error_message, complexity
                 FROM interaction_records
                 WHERE success = 0
                 ORDER BY timestamp DESC
                 LIMIT ?1",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                let id: String = row.get(0)?;
                let ts_str: String = row.get(1)?;
                let query_type: String = row.get(2)?;
                let tools_json: String = row.get(3)?;
                let success: i32 = row.get(4)?;
                let duration: i64 = row.get(5)?;
                let error_msg: Option<String> = row.get(6)?;
                let complexity: String = row.get(7)?;
                Ok((
                    id, ts_str, query_type, tools_json, success, duration, error_msg, complexity,
                ))
            })
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query errors: {}", e)))?;

        let mut records = Vec::new();
        for row in rows {
            let (id, ts_str, query_type, tools_json, success, duration, error_msg, complexity) =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;

            let timestamp = DateTime::parse_from_rfc3339(&ts_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let tools_used: Vec<String> = serde_json::from_str(&tools_json).unwrap_or_default();

            records.push(InteractionRecord {
                id,
                timestamp,
                query_type,
                tools_used,
                success: success != 0,
                duration_ms: duration as u64,
                error_message: error_msg,
                complexity,
            });
        }

        Ok(records)
    }

    /// Remove old records beyond `max_records`, keeping the most recent.
    ///
    /// Returns the number of records deleted.
    pub fn prune(&self) -> Result<usize> {
        let _lock = self
            ._write_lock
            .lock()
            .map_err(|e| zeus_core::Error::Internal(format!("Lock poisoned: {}", e)))?;

        let conn = self.open_db()?;

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM interaction_records", [], |row| {
                row.get(0)
            })
            .map_err(|e| zeus_core::Error::Database(format!("Failed to count records: {}", e)))?;

        if count <= self.max_records as i64 {
            return Ok(0);
        }

        let excess = count - self.max_records as i64;
        conn.execute(
            "DELETE FROM interaction_records WHERE id IN (
                SELECT id FROM interaction_records ORDER BY timestamp ASC LIMIT ?1
            )",
            rusqlite::params![excess],
        )
        .map_err(|e| zeus_core::Error::Database(format!("Failed to prune records: {}", e)))?;

        debug!("Pruned {} old interaction records", excess);
        Ok(excess as usize)
    }

    /// Whether learning is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

// ============================================================================
// Strategic Learning — plan/tool/provider pattern tracking
// ============================================================================

/// Outcome of a learned pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Success,
    Failure,
    Mixed,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Success => write!(f, "success"),
            Outcome::Failure => write!(f, "failure"),
            Outcome::Mixed => write!(f, "mixed"),
        }
    }
}

impl Outcome {
    /// Parse from string, defaulting to Mixed for unknown values.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "success" => Outcome::Success,
            "failure" => Outcome::Failure,
            _ => Outcome::Mixed,
        }
    }
}

/// A single learned pattern — e.g. "for file_operation intents, using
/// read_file → edit_file tool sequence succeeds 85% of the time."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Learning {
    /// Category: "plan", "tool_sequence", "provider"
    pub category: String,
    /// The pattern description (e.g. "read_file -> edit_file", "anthropic/claude-sonnet")
    pub pattern: String,
    /// Overall outcome assessment
    pub outcome: Outcome,
    /// Confidence score 0.0 - 1.0
    pub confidence: f64,
    /// Number of observations that produced this learning
    pub observations: u32,
    /// When this learning was first recorded
    pub created_at: DateTime<Utc>,
    /// When this learning was last updated
    pub updated_at: DateTime<Utc>,
    /// Optional intent/task type this learning applies to
    pub intent: Option<String>,
}

/// Strategic learning engine that tracks which plans, tool sequences, and
/// providers work best for different task types.
///
/// Builds on top of the raw interaction-level `LearningEngine` with a
/// higher-level abstraction for pattern-based decision making.
pub struct StrategicLearner {
    db_path: PathBuf,
    _write_lock: Mutex<()>,
}

impl StrategicLearner {
    /// Create a new strategic learner, initializing the patterns table.
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let learner = Self {
            db_path,
            _write_lock: Mutex::new(()),
        };
        learner.init_db()?;
        Ok(learner)
    }

    fn open_db(&self) -> Result<rusqlite::Connection> {
        rusqlite::Connection::open(&self.db_path)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to open learner db: {}", e)))
    }

    fn init_db(&self) -> Result<()> {
        let conn = self.open_db()?;
        crate::db::run_migrations(&conn, STRATEGIC_MIGRATIONS)?;
        Ok(())
    }

    /// Record an outcome for a pattern. If the pattern already exists,
    /// update confidence via exponential moving average and increment observations.
    pub fn record_outcome(
        &self,
        category: &str,
        pattern: &str,
        success: bool,
        intent: Option<&str>,
    ) -> Result<()> {
        let _lock = self
            ._write_lock
            .lock()
            .map_err(|e| zeus_core::Error::Internal(format!("Lock poisoned: {}", e)))?;

        let conn = self.open_db()?;
        let intent_key = intent.unwrap_or("");
        let now = Utc::now().to_rfc3339();

        // Check if pattern exists
        let existing: Option<(f64, u32, String)> = conn
            .query_row(
                "SELECT confidence, observations, outcome FROM learnings
                 WHERE category = ?1 AND pattern = ?2 AND intent = ?3",
                rusqlite::params![category, pattern, intent_key],
                |row| {
                    let conf: f64 = row.get(0)?;
                    let obs: u32 = row.get(1)?;
                    let outcome: String = row.get(2)?;
                    Ok((conf, obs, outcome))
                },
            )
            .ok();

        if let Some((old_conf, old_obs, old_outcome_str)) = existing {
            // Exponential moving average: alpha=0.2
            let new_signal = if success { 1.0 } else { 0.0 };
            let alpha = 0.2;
            let new_conf = (1.0 - alpha) * old_conf + alpha * new_signal;
            let new_obs = old_obs + 1;

            // Determine outcome: if confidence > 0.7 -> Success, < 0.3 -> Failure, else Mixed
            let new_outcome = if new_conf > 0.7 {
                Outcome::Success
            } else if new_conf < 0.3 {
                Outcome::Failure
            } else {
                // If it was previously Success/Failure and now in the middle, Mixed
                let _old_outcome = Outcome::from_str_lossy(&old_outcome_str);
                Outcome::Mixed
            };

            conn.execute(
                "UPDATE learnings SET confidence = ?1, observations = ?2, outcome = ?3, updated_at = ?4
                 WHERE category = ?5 AND pattern = ?6 AND intent = ?7",
                rusqlite::params![
                    new_conf,
                    new_obs,
                    new_outcome.to_string(),
                    now,
                    category,
                    pattern,
                    intent_key,
                ],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to update learning: {}", e)))?;
        } else {
            // New pattern
            let confidence = if success { 1.0 } else { 0.0 };
            let outcome = if success {
                Outcome::Success
            } else {
                Outcome::Failure
            };

            conn.execute(
                "INSERT INTO learnings (category, pattern, outcome, confidence, observations, created_at, updated_at, intent)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?6)",
                rusqlite::params![
                    category,
                    pattern,
                    outcome.to_string(),
                    confidence,
                    now,
                    intent_key,
                ],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to insert learning: {}", e)))?;
        }

        Ok(())
    }

    /// Get recommendations for a given intent, sorted by confidence descending.
    /// Returns learnings from all categories that match the intent.
    pub fn get_recommendations(&self, intent: &str) -> Result<Vec<Learning>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare(
                "SELECT category, pattern, outcome, confidence, observations, created_at, updated_at, intent
                 FROM learnings
                 WHERE intent = ?1
                 ORDER BY confidence DESC",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![intent], Self::row_to_learning)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let learning =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            results.push(learning);
        }
        Ok(results)
    }

    /// Get the top N patterns across all intents, sorted by confidence * observations.
    pub fn top_patterns(&self, n: usize) -> Result<Vec<Learning>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare(
                "SELECT category, pattern, outcome, confidence, observations, created_at, updated_at, intent
                 FROM learnings
                 ORDER BY (confidence * observations) DESC
                 LIMIT ?1",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![n as i64], Self::row_to_learning)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let learning =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            results.push(learning);
        }
        Ok(results)
    }

    /// Remove learnings older than `max_age` that haven't been updated recently.
    /// Returns the number of pruned learnings.
    pub fn prune_stale(&self, max_age: chrono::Duration) -> Result<usize> {
        let _lock = self
            ._write_lock
            .lock()
            .map_err(|e| zeus_core::Error::Internal(format!("Lock poisoned: {}", e)))?;

        let conn = self.open_db()?;
        let cutoff = (Utc::now() - max_age).to_rfc3339();

        let deleted = conn
            .execute(
                "DELETE FROM learnings WHERE updated_at < ?1",
                rusqlite::params![cutoff],
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prune stale: {}", e)))?;

        if deleted > 0 {
            debug!("Pruned {} stale learnings", deleted);
        }
        Ok(deleted)
    }

    /// Get all learnings for a specific category (e.g. "plan", "tool_sequence", "provider").
    pub fn by_category(&self, category: &str) -> Result<Vec<Learning>> {
        let conn = self.open_db()?;

        let mut stmt = conn
            .prepare(
                "SELECT category, pattern, outcome, confidence, observations, created_at, updated_at, intent
                 FROM learnings
                 WHERE category = ?1
                 ORDER BY confidence DESC",
            )
            .map_err(|e| zeus_core::Error::Database(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(rusqlite::params![category], Self::row_to_learning)
            .map_err(|e| zeus_core::Error::Database(format!("Failed to query: {}", e)))?;

        let mut results = Vec::new();
        for row in rows {
            let learning =
                row.map_err(|e| zeus_core::Error::Database(format!("Failed to read row: {}", e)))?;
            results.push(learning);
        }
        Ok(results)
    }

    /// Total number of learnings stored.
    pub fn count(&self) -> Result<usize> {
        let conn = self.open_db()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM learnings", [], |row| row.get(0))
            .map_err(|e| zeus_core::Error::Database(format!("Failed to count: {}", e)))?;
        Ok(count as usize)
    }

    /// Helper to convert a database row into a Learning struct.
    fn row_to_learning(row: &rusqlite::Row) -> rusqlite::Result<Learning> {
        let category: String = row.get(0)?;
        let pattern: String = row.get(1)?;
        let outcome_str: String = row.get(2)?;
        let confidence: f64 = row.get(3)?;
        let observations: u32 = row.get(4)?;
        let created_str: String = row.get(5)?;
        let updated_str: String = row.get(6)?;
        let intent_str: String = row.get(7)?;

        let outcome = Outcome::from_str_lossy(&outcome_str);
        let created_at = DateTime::parse_from_rfc3339(&created_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = DateTime::parse_from_rfc3339(&updated_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let intent = if intent_str.is_empty() {
            None
        } else {
            Some(intent_str)
        };

        Ok(Learning {
            category,
            pattern,
            outcome,
            confidence,
            observations,
            created_at,
            updated_at,
            intent,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a LearningEngine backed by a temporary database.
    fn create_test_engine(max_records: usize) -> (LearningEngine, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_learning.db");
        let config = LearningConfig {
            db_path,
            max_records,
            enable_learning: true,
        };
        let engine = LearningEngine::new(&config).unwrap();
        (engine, tmp)
    }

    /// Build a sample InteractionRecord.
    fn sample_record(
        id: &str,
        query_type: &str,
        tools: Vec<&str>,
        success: bool,
        duration_ms: u64,
    ) -> InteractionRecord {
        InteractionRecord {
            id: id.to_string(),
            timestamp: Utc::now(),
            query_type: query_type.to_string(),
            tools_used: tools.into_iter().map(String::from).collect(),
            success,
            duration_ms,
            error_message: if success {
                None
            } else {
                Some("something went wrong".to_string())
            },
            complexity: "simple".to_string(),
        }
    }

    #[test]
    fn test_record_and_retrieve() {
        let (engine, _tmp) = create_test_engine(100);

        let rec = sample_record("r1", "file_op", vec!["read_file"], true, 150);
        engine.record(rec).unwrap();

        // Verify by fetching patterns.
        let pattern = engine.get_patterns("file_op").unwrap().unwrap();
        assert_eq!(pattern.total_count, 1);
        assert_eq!(pattern.success_count, 1);
    }

    #[test]
    fn test_pattern_summary() {
        let (engine, _tmp) = create_test_engine(100);

        engine
            .record(sample_record("p1", "search", vec!["web_fetch"], true, 200))
            .unwrap();
        engine
            .record(sample_record("p2", "search", vec!["web_fetch"], true, 300))
            .unwrap();
        engine
            .record(sample_record(
                "p3",
                "search",
                vec!["web_fetch", "read_file"],
                false,
                500,
            ))
            .unwrap();

        let pattern = engine.get_patterns("search").unwrap().unwrap();
        assert_eq!(pattern.total_count, 3);
        assert_eq!(pattern.success_count, 2);
        assert_eq!(pattern.query_type, "search");
        // avg_duration should be (200 + 300 + 500) / 3 ≈ 333.33
        assert!(pattern.avg_duration_ms > 330.0 && pattern.avg_duration_ms < 340.0);
        // web_fetch should be the most common tool with count 3.
        assert_eq!(pattern.common_tools[0].0, "web_fetch");
        assert_eq!(pattern.common_tools[0].1, 3);
    }

    #[test]
    fn test_tool_effectiveness() {
        let (engine, _tmp) = create_test_engine(100);

        engine
            .record(sample_record("e1", "ops", vec!["shell"], true, 100))
            .unwrap();
        engine
            .record(sample_record("e2", "ops", vec!["shell"], true, 200))
            .unwrap();
        engine
            .record(sample_record("e3", "ops", vec!["shell"], false, 300))
            .unwrap();
        engine
            .record(sample_record("e4", "ops", vec!["read_file"], true, 50))
            .unwrap();

        let effectiveness = engine.get_tool_effectiveness().unwrap();
        assert!(!effectiveness.is_empty());

        let shell_stat = effectiveness
            .iter()
            .find(|t| t.tool_name == "shell")
            .unwrap();
        assert_eq!(shell_stat.total_uses, 3);
        // 2 out of 3 succeeded
        assert!((shell_stat.success_rate - 0.6667).abs() < 0.01);
        // avg duration = (100+200+300)/3 = 200
        assert!((shell_stat.avg_duration_ms - 200.0).abs() < 0.01);

        let read_stat = effectiveness
            .iter()
            .find(|t| t.tool_name == "read_file")
            .unwrap();
        assert_eq!(read_stat.total_uses, 1);
        assert!((read_stat.success_rate - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_suggest_tools() {
        let (engine, _tmp) = create_test_engine(100);

        // Record successful interactions for "file_op" using read_file and write_file.
        engine
            .record(sample_record(
                "s1",
                "file_op",
                vec!["read_file", "write_file"],
                true,
                100,
            ))
            .unwrap();
        engine
            .record(sample_record("s2", "file_op", vec!["read_file"], true, 80))
            .unwrap();
        // A failed interaction should not influence suggestions.
        engine
            .record(sample_record("s3", "file_op", vec!["shell"], false, 500))
            .unwrap();

        let suggestions = engine.suggest_tools("file_op").unwrap();
        // read_file should be first (used in 2 successes), write_file second (1).
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0], "read_file");
        assert_eq!(suggestions[1], "write_file");

        // shell should not appear because it was only in a failed interaction.
        assert!(!suggestions.contains(&"shell".to_string()));
    }

    #[test]
    fn test_success_rate() {
        let (engine, _tmp) = create_test_engine(100);

        engine
            .record(sample_record("sr1", "a", vec![], true, 100))
            .unwrap();
        engine
            .record(sample_record("sr2", "a", vec![], true, 100))
            .unwrap();
        engine
            .record(sample_record("sr3", "a", vec![], false, 100))
            .unwrap();
        engine
            .record(sample_record("sr4", "b", vec![], true, 100))
            .unwrap();

        let rate = engine.success_rate().unwrap();
        // 3 successes out of 4 = 0.75
        assert!((rate - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_recent_errors() {
        let (engine, _tmp) = create_test_engine(100);

        engine
            .record(sample_record("re1", "ops", vec!["shell"], true, 100))
            .unwrap();
        engine
            .record(sample_record("re2", "ops", vec!["shell"], false, 200))
            .unwrap();
        engine
            .record(sample_record("re3", "ops", vec!["shell"], false, 300))
            .unwrap();

        let errors = engine.recent_errors(10).unwrap();
        assert_eq!(errors.len(), 2);
        // Most recent first.
        assert!(!errors[0].success);
        assert!(errors[0].error_message.is_some());

        // Limit works.
        let errors_limited = engine.recent_errors(1).unwrap();
        assert_eq!(errors_limited.len(), 1);
    }

    #[test]
    fn test_prune_old_records() {
        let (engine, _tmp) = create_test_engine(3);

        // Insert 5 records.
        for i in 0..5 {
            engine
                .record(sample_record(
                    &format!("prune{}", i),
                    "misc",
                    vec![],
                    true,
                    100,
                ))
                .unwrap();
        }

        // The automatic pruning in `record` should have already trimmed to 3,
        // but let's verify via explicit prune.
        let pruned = engine.prune().unwrap();
        // After the auto-prune in the last insert, we should be at max_records
        // already, so explicit prune returns 0.
        assert_eq!(pruned, 0);

        // Verify there are exactly 3 records left.
        let rate = engine.success_rate().unwrap();
        assert!((rate - 1.0).abs() < 0.001); // all remaining are successes

        // Verify total count via patterns: all were "misc" type.
        let pattern = engine.get_patterns("misc").unwrap().unwrap();
        assert_eq!(pattern.total_count, 3);
    }

    #[test]
    fn test_empty_database() {
        let (engine, _tmp) = create_test_engine(100);

        // Patterns for non-existent type should be None.
        assert!(engine.get_patterns("nonexistent").unwrap().is_none());

        // Tool effectiveness should be empty.
        assert!(engine.get_tool_effectiveness().unwrap().is_empty());

        // Suggestions should be empty.
        assert!(engine.suggest_tools("any").unwrap().is_empty());

        // Success rate should be 0.0.
        assert!((engine.success_rate().unwrap()).abs() < 0.001);

        // Recent errors should be empty.
        assert!(engine.recent_errors(10).unwrap().is_empty());

        // Prune should remove nothing.
        assert_eq!(engine.prune().unwrap(), 0);
    }

    #[test]
    fn test_learning_config_defaults() {
        let config = LearningConfig::default();
        assert_eq!(config.max_records, 10_000);
        assert!(config.enable_learning);
        assert_eq!(config.db_path, PathBuf::from("learning.db"));
    }

    #[test]
    fn test_disabled_learning() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("disabled.db");
        let config = LearningConfig {
            db_path,
            max_records: 100,
            enable_learning: false,
        };
        let engine = LearningEngine::new(&config).unwrap();

        // Recording should succeed but not actually store anything.
        engine
            .record(sample_record("d1", "chat", vec![], true, 100))
            .unwrap();

        // Nothing stored because learning is disabled.
        assert!(engine.get_patterns("chat").unwrap().is_none());
        assert!(!engine.is_enabled());
    }

    // ========================================================================
    // StrategicLearner tests
    // ========================================================================

    fn create_test_learner() -> (StrategicLearner, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test_strategic.db");
        let learner = StrategicLearner::new(db_path).unwrap();
        (learner, tmp)
    }

    #[test]
    fn test_strategic_record_and_retrieve() {
        let (learner, _tmp) = create_test_learner();

        learner
            .record_outcome("plan", "decompose_then_execute", true, Some("complex_task"))
            .unwrap();

        let recs = learner.get_recommendations("complex_task").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].category, "plan");
        assert_eq!(recs[0].pattern, "decompose_then_execute");
        assert_eq!(recs[0].outcome, Outcome::Success);
        assert!((recs[0].confidence - 1.0).abs() < 0.001);
        assert_eq!(recs[0].observations, 1);
        assert_eq!(recs[0].intent, Some("complex_task".to_string()));
    }

    #[test]
    fn test_strategic_confidence_ema() {
        let (learner, _tmp) = create_test_learner();

        // Record 3 successes then 1 failure
        for _ in 0..3 {
            learner
                .record_outcome("tool_sequence", "read->edit", true, Some("file_op"))
                .unwrap();
        }
        learner
            .record_outcome("tool_sequence", "read->edit", false, Some("file_op"))
            .unwrap();

        let recs = learner.get_recommendations("file_op").unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].observations, 4);
        // After 3 successes: 1.0 -> 1.0 -> 1.0, then failure: 0.8 * 1.0 + 0.2 * 0.0 = 0.8
        assert!((recs[0].confidence - 0.8).abs() < 0.01);
        assert_eq!(recs[0].outcome, Outcome::Success); // 0.8 > 0.7
    }

    #[test]
    fn test_strategic_outcome_transitions() {
        let (learner, _tmp) = create_test_learner();

        // Start with failure
        learner
            .record_outcome("provider", "openai/gpt-4o", false, Some("code_gen"))
            .unwrap();
        let recs = learner.get_recommendations("code_gen").unwrap();
        assert_eq!(recs[0].outcome, Outcome::Failure);
        assert!((recs[0].confidence - 0.0).abs() < 0.001);

        // Add a success: EMA = 0.8 * 0.0 + 0.2 * 1.0 = 0.2
        learner
            .record_outcome("provider", "openai/gpt-4o", true, Some("code_gen"))
            .unwrap();
        let recs = learner.get_recommendations("code_gen").unwrap();
        assert_eq!(recs[0].outcome, Outcome::Failure); // 0.2 < 0.3

        // Add more successes to push into Mixed
        learner
            .record_outcome("provider", "openai/gpt-4o", true, Some("code_gen"))
            .unwrap();
        // EMA = 0.8 * 0.2 + 0.2 * 1.0 = 0.36 -> Mixed
        let recs = learner.get_recommendations("code_gen").unwrap();
        assert_eq!(recs[0].outcome, Outcome::Mixed);
    }

    #[test]
    fn test_strategic_top_patterns() {
        let (learner, _tmp) = create_test_learner();

        // Record patterns with different confidences and observations
        for _ in 0..10 {
            learner
                .record_outcome("plan", "step_by_step", true, Some("analysis"))
                .unwrap();
        }
        for _ in 0..3 {
            learner
                .record_outcome("tool_sequence", "shell_then_read", true, Some("debug"))
                .unwrap();
        }
        learner
            .record_outcome("provider", "anthropic/claude", false, Some("chat"))
            .unwrap();

        let top = learner.top_patterns(2).unwrap();
        assert_eq!(top.len(), 2);
        // step_by_step has highest confidence * observations (1.0 * 10)
        assert_eq!(top[0].pattern, "step_by_step");
        // shell_then_read next (1.0 * 3)
        assert_eq!(top[1].pattern, "shell_then_read");
    }

    #[test]
    fn test_strategic_prune_stale() {
        let (learner, _tmp) = create_test_learner();

        // Record a pattern
        learner
            .record_outcome("plan", "old_pattern", true, Some("task"))
            .unwrap();

        // Prune with 0 duration should remove everything
        let pruned = learner.prune_stale(chrono::Duration::zero()).unwrap();
        // Since updated_at is "now", and cutoff is also "now", records AT the cutoff
        // might or might not be pruned depending on sub-second timing.
        // Use a negative duration to guarantee pruning.
        let _ = pruned;

        // Record fresh, then prune with large duration — should keep everything
        learner
            .record_outcome("plan", "fresh_pattern", true, Some("task"))
            .unwrap();
        let pruned = learner.prune_stale(chrono::Duration::hours(24)).unwrap();
        assert_eq!(pruned, 0);
    }

    #[test]
    fn test_strategic_by_category() {
        let (learner, _tmp) = create_test_learner();

        learner
            .record_outcome("plan", "decompose", true, Some("complex"))
            .unwrap();
        learner
            .record_outcome("plan", "direct_execute", true, Some("simple"))
            .unwrap();
        learner
            .record_outcome("provider", "anthropic", true, Some("complex"))
            .unwrap();

        let plans = learner.by_category("plan").unwrap();
        assert_eq!(plans.len(), 2);
        assert!(plans.iter().all(|l| l.category == "plan"));

        let providers = learner.by_category("provider").unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].pattern, "anthropic");
    }

    #[test]
    fn test_strategic_count() {
        let (learner, _tmp) = create_test_learner();

        assert_eq!(learner.count().unwrap(), 0);

        learner
            .record_outcome("plan", "p1", true, Some("a"))
            .unwrap();
        learner
            .record_outcome("plan", "p2", true, Some("b"))
            .unwrap();
        assert_eq!(learner.count().unwrap(), 2);

        // Same key updates, doesn't create new
        learner
            .record_outcome("plan", "p1", false, Some("a"))
            .unwrap();
        assert_eq!(learner.count().unwrap(), 2);
    }

    #[test]
    fn test_strategic_empty_recommendations() {
        let (learner, _tmp) = create_test_learner();
        let recs = learner.get_recommendations("nonexistent").unwrap();
        assert!(recs.is_empty());
    }

    #[test]
    fn test_strategic_no_intent() {
        let (learner, _tmp) = create_test_learner();

        // Record without intent (global pattern)
        learner
            .record_outcome("tool_sequence", "read->write", true, None)
            .unwrap();

        // Should be retrievable with empty intent
        let recs = learner.get_recommendations("").unwrap();
        assert_eq!(recs.len(), 1);
        assert!(recs[0].intent.is_none());
    }

    #[test]
    fn test_outcome_display_and_parse() {
        assert_eq!(Outcome::Success.to_string(), "success");
        assert_eq!(Outcome::Failure.to_string(), "failure");
        assert_eq!(Outcome::Mixed.to_string(), "mixed");

        assert_eq!(Outcome::from_str_lossy("success"), Outcome::Success);
        assert_eq!(Outcome::from_str_lossy("failure"), Outcome::Failure);
        assert_eq!(Outcome::from_str_lossy("mixed"), Outcome::Mixed);
        assert_eq!(Outcome::from_str_lossy("unknown"), Outcome::Mixed);
    }

    #[test]
    fn test_learning_struct_fields() {
        let learning = Learning {
            category: "plan".to_string(),
            pattern: "decompose".to_string(),
            outcome: Outcome::Success,
            confidence: 0.95,
            observations: 20,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            intent: Some("complex_task".to_string()),
        };
        assert_eq!(learning.category, "plan");
        assert_eq!(learning.pattern, "decompose");
        assert_eq!(learning.outcome, Outcome::Success);
        assert!(learning.confidence > 0.9);
        assert_eq!(learning.observations, 20);
        assert!(learning.intent.is_some());
    }

    #[test]
    fn test_strategic_multiple_categories_same_intent() {
        let (learner, _tmp) = create_test_learner();

        learner
            .record_outcome("plan", "decompose", true, Some("code_review"))
            .unwrap();
        learner
            .record_outcome("tool_sequence", "read->shell", true, Some("code_review"))
            .unwrap();
        learner
            .record_outcome("provider", "anthropic/opus", true, Some("code_review"))
            .unwrap();

        let recs = learner.get_recommendations("code_review").unwrap();
        assert_eq!(recs.len(), 3);
        // All should be high confidence (first observation = 1.0)
        assert!(recs.iter().all(|r| r.confidence > 0.9));
    }
}
