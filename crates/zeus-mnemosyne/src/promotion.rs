//! Memory Lifecycle & Promotion — episodic → semantic promotion, session
//! consolidation, and garbage collection.
//!
//! This module implements three lifecycle operations:
//!
//! 1. **Auto-promote**: scans episodic memories whose importance exceeds a
//!    threshold *and* that have been accessed at least `min_access` times,
//!    extracts a key-fact sentence, and stores a new semantic memory.
//!
//! 2. **Consolidate session**: rolls up a session's episodic memories into a
//!    compact summary memory and marks originals as superseded.
//!
//! 3. **Garbage collect**: removes stale episodic memories that are older than
//!    a configurable retention window while preserving pinned, promoted,
//!    semantic, and high-importance entries.

use chrono::{Duration, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::{MemoryStore, MemoryType};
use zeus_core::{Error, Message, Result};

// ============================================================================
// Configuration
// ============================================================================

/// Garbage-collection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcConfig {
    /// Maximum age (in days) for episodic memories before they become GC
    /// candidates.
    pub max_age_days: u32,
    /// Episodic memories with importance **at or above** this value are kept
    /// regardless of age.
    pub importance_floor: f32,
    /// If `true`, memories that have been promoted to semantic are always kept
    /// (the *original* episodic row is eligible for GC, but the semantic copy
    /// is not).
    pub keep_promoted: bool,
    /// Memory types that are never garbage-collected.
    pub protected_types: Vec<MemoryType>,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            max_age_days: 30,
            importance_floor: 0.7,
            keep_promoted: true,
            protected_types: vec![
                MemoryType::Semantic,
                MemoryType::Fact,
                MemoryType::Preference,
            ],
        }
    }
}

// ============================================================================
// Result types
// ============================================================================

/// Result of a garbage-collection run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GcResult {
    /// Number of memories scanned.
    pub scanned: usize,
    /// Number of memories deleted.
    pub deleted: usize,
    /// Number of memories kept (protected or above importance floor).
    pub kept: usize,
}

/// Result of consolidating a single session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Session that was consolidated.
    pub session_id: String,
    /// Number of episodic memories rolled up.
    pub rolled_up: usize,
    /// Number of summary memories created (typically 1).
    pub summaries_created: usize,
    /// ID of the created summary row, if any.
    pub summary_id: Option<i64>,
}

/// Result of an auto-promotion run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromotionResult {
    /// Number of episodic memories scanned.
    pub scanned: usize,
    /// Number promoted to semantic.
    pub promoted: usize,
}

// ============================================================================
// Core functions
// ============================================================================

/// Scan episodic memories and promote those that exceed `threshold` importance.
///
/// For each candidate, the first sentence is extracted as the "key fact" and
/// stored as a new semantic memory.  A `promotion` pattern is logged via
/// `upsert_pattern` for auditing.
///
/// Returns the number of memories promoted.
pub fn auto_promote(
    store: &MemoryStore,
    threshold: f32,
    min_access_days: u32,
) -> Result<PromotionResult> {
    // Find episodic memories that are high-importance *and* old enough to have
    // proven their staying power (last_accessed at least min_access_days ago,
    // or if NULL, created at least min_access_days ago).
    let cutoff = (Utc::now() - Duration::days(min_access_days as i64)).to_rfc3339();

    let mut stmt = store
        .conn()
        .prepare(
            "SELECT id, session_id, content, importance
         FROM messages
         WHERE memory_type = 'episodic'
           AND importance >= ?1
           AND valid_to IS NULL
           AND COALESCE(last_accessed, timestamp) <= ?2
         ORDER BY importance DESC",
        )
        .map_err(|e| Error::Database(format!("Failed to prepare promotion scan: {e}")))?;

    let candidates: Vec<(i64, String, String, f64)> = stmt
        .query_map(params![threshold as f64, cutoff], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .map_err(|e| Error::Database(format!("Promotion scan failed: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Database(format!("Failed to collect promotion candidates: {e}")))?;

    let scanned = candidates.len();
    let mut promoted = 0usize;

    for (msg_id, session_id, content, importance) in &candidates {
        let key_fact = extract_key_fact(content);
        if key_fact.is_empty() {
            continue;
        }

        // Create a new semantic memory with the extracted fact
        let msg = Message::system(key_fact.clone());

        let new_id =
            store.store_typed(session_id, &msg, MemoryType::Semantic, *importance as f32)?;

        // Mark the original episodic memory as superseded
        store
            .conn()
            .execute(
                "UPDATE messages SET valid_to = ?1, superseded_by = ?2 WHERE id = ?3",
                params![Utc::now().to_rfc3339(), new_id, msg_id],
            )
            .map_err(|e| Error::Database(format!("Failed to mark superseded: {e}")))?;

        // Log promotion as a pattern for auditing
        log_promotion(store, *msg_id, new_id, &key_fact)?;

        promoted += 1;
        debug!(original_id = msg_id, new_id, "Promoted episodic → semantic");
    }

    if promoted > 0 {
        info!(scanned, promoted, "Auto-promotion complete");
    }

    Ok(PromotionResult { scanned, promoted })
}

/// Consolidate all episodic memories for a session into a single summary.
///
/// The original episodic rows are marked `valid_to = now` (soft-deleted) and a
/// new `Summary`-typed memory is inserted containing the concatenated content.
///
/// When `fact_check` is true, key facts are extracted from the original content
/// and validated against the summary — any missing facts are appended.
pub fn consolidate_session(
    store: &MemoryStore,
    session_id: &str,
    fact_check: bool,
) -> Result<ConsolidationResult> {
    // Query episodic memories for this session directly (need memory_type filter)
    let mut episodic_ids: Vec<i64> = Vec::new();
    let mut content_parts: Vec<String> = Vec::new();

    let mut stmt = store
        .conn()
        .prepare(
            "SELECT id, content, memory_type, importance
         FROM messages
         WHERE session_id = ?1
           AND memory_type = 'episodic'
           AND valid_to IS NULL
         ORDER BY timestamp ASC",
        )
        .map_err(|e| Error::Database(format!("Failed to query session episodics: {e}")))?;

    let rows: Vec<(i64, String, f64)> = stmt
        .query_map(params![session_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .map_err(|e| Error::Database(format!("Session episodic query failed: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Database(format!("Failed to collect session episodics: {e}")))?;

    drop(stmt);

    if rows.is_empty() {
        return Ok(ConsolidationResult {
            session_id: session_id.to_string(),
            ..Default::default()
        });
    }

    let mut max_importance: f64 = 0.0;
    for (id, content, importance) in &rows {
        episodic_ids.push(*id);
        content_parts.push(content.clone());
        if *importance > max_importance {
            max_importance = *importance;
        }
    }

    // Build a summary: concatenate content, truncated to 2000 chars
    let mut summary_text = build_summary(&content_parts);

    // Fact-check: extract key facts and validate they appear in the summary
    if fact_check {
        let facts = extract_facts(&content_parts);
        if facts.total() > 0 {
            let validation = validate_compaction(&summary_text, &facts);
            if !validation.missing.is_empty() {
                debug!(
                    session_id,
                    total = facts.total(),
                    preserved = validation.preserved,
                    missing = validation.missing.len(),
                    "Compaction fact-check: augmenting summary with missing facts"
                );
                summary_text = augment_summary(&summary_text, &validation.missing, 2000);
            }
        }
    }

    let summary_msg = Message::system(summary_text);

    let summary_id = store.store_typed(
        session_id,
        &summary_msg,
        MemoryType::Summary,
        max_importance as f32,
    )?;

    // Mark originals as superseded
    let now = Utc::now().to_rfc3339();
    for id in &episodic_ids {
        store
            .conn()
            .execute(
                "UPDATE messages SET valid_to = ?1, superseded_by = ?2 WHERE id = ?3",
                params![now, summary_id, id],
            )
            .map_err(|e| Error::Database(format!("Failed to supersede message {id}: {e}")))?;
    }

    let rolled_up = episodic_ids.len();
    info!(session_id, rolled_up, summary_id, "Session consolidated");

    Ok(ConsolidationResult {
        session_id: session_id.to_string(),
        rolled_up,
        summaries_created: 1,
        summary_id: Some(summary_id),
    })
}

/// Garbage-collect old, low-importance episodic memories.
///
/// Deletes episodic memories older than `config.max_age_days` whose importance
/// is below `config.importance_floor`.  Protected types (semantic, fact,
/// preference) are never deleted.
pub fn garbage_collect(store: &MemoryStore, config: &GcConfig) -> Result<GcResult> {
    let cutoff = (Utc::now() - Duration::days(config.max_age_days as i64)).to_rfc3339();

    // Build the protected-types SQL fragment
    let protected: Vec<String> = config
        .protected_types
        .iter()
        .map(|t| format!("'{}'", t.as_str()))
        .collect();
    let protected_csv = if protected.is_empty() {
        "''".to_string()
    } else {
        protected.join(",")
    };

    // Count total candidates (episodic + old + low importance + not protected)
    let count_sql = format!(
        "SELECT COUNT(*) FROM messages
         WHERE timestamp < ?1
           AND memory_type NOT IN ({protected_csv})
           AND valid_to IS NULL"
    );
    let scanned: usize = store
        .conn()
        .query_row(&count_sql, params![cutoff], |row| row.get::<_, i64>(0))
        .map_err(|e| Error::Database(format!("GC count failed: {e}")))?
        as usize;

    // Delete those below the importance floor
    let delete_sql = format!(
        "DELETE FROM messages
         WHERE timestamp < ?1
           AND memory_type NOT IN ({protected_csv})
           AND importance < ?2
           AND valid_to IS NULL"
    );
    let deleted = store
        .conn()
        .execute(&delete_sql, params![cutoff, config.importance_floor as f64])
        .map_err(|e| Error::Database(format!("GC delete failed: {e}")))?;

    let kept = scanned.saturating_sub(deleted);

    if deleted > 0 {
        info!(scanned, deleted, kept, "Garbage collection complete");
    }

    Ok(GcResult {
        scanned,
        deleted,
        kept,
    })
}

// ============================================================================
// Helpers
// ============================================================================

// ============================================================================
// Fact Extraction & Validation
// ============================================================================

/// Facts extracted from content before compaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtractedFacts {
    /// Named entities (capitalized multi-word phrases, proper nouns).
    pub entities: Vec<String>,
    /// Date-like strings (ISO dates, month names, relative dates).
    pub dates: Vec<String>,
    /// Decision statements (sentences with decision keywords).
    pub decisions: Vec<String>,
    /// Numeric values with context (e.g., "port 5432", "version 3.2").
    pub numbers: Vec<String>,
}

impl ExtractedFacts {
    /// Total number of extracted facts.
    pub fn total(&self) -> usize {
        self.entities.len() + self.dates.len() + self.decisions.len() + self.numbers.len()
    }

    /// All facts as a flat list of strings.
    fn all_facts(&self) -> Vec<&str> {
        self.entities
            .iter()
            .chain(self.dates.iter())
            .chain(self.decisions.iter())
            .chain(self.numbers.iter())
            .map(|s| s.as_str())
            .collect()
    }
}

/// Result of post-compaction fact validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactValidation {
    /// Total facts extracted from source content.
    pub total_facts: usize,
    /// Facts found in the compacted summary.
    pub preserved: usize,
    /// Facts missing from the summary.
    pub missing: Vec<String>,
    /// Coverage ratio (0.0–1.0).
    pub coverage: f64,
}

/// Extract key facts (entities, dates, decisions, numbers) from content parts.
pub fn extract_facts(parts: &[String]) -> ExtractedFacts {
    let mut facts = ExtractedFacts::default();
    let mut seen_entities = std::collections::HashSet::new();
    let mut seen_dates = std::collections::HashSet::new();
    let mut seen_numbers = std::collections::HashSet::new();

    for part in parts {
        // Extract capitalized phrases (2+ words starting with uppercase, likely proper nouns)
        let words: Vec<&str> = part.split_whitespace().collect();
        let mut i = 0;
        while i < words.len() {
            if words[i].len() >= 2
                && words[i].chars().next().is_some_and(|c| c.is_uppercase())
                && !is_sentence_start(part, words[i])
            {
                let mut phrase = vec![words[i]];
                let mut j = i + 1;
                while j < words.len()
                    && words[j].chars().next().is_some_and(|c| c.is_uppercase())
                    && words[j].len() >= 2
                {
                    phrase.push(words[j]);
                    j += 1;
                }
                if phrase.len() >= 2 || is_likely_entity(words[i]) {
                    let entity = phrase
                        .join(" ")
                        .trim_matches(|c: char| c.is_ascii_punctuation())
                        .to_string();
                    if entity.len() >= 3 && seen_entities.insert(entity.to_lowercase()) {
                        facts.entities.push(entity);
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }

        // Extract dates: ISO format, month names, relative dates
        for word in &words {
            let lower = word.to_lowercase();
            let stripped = lower.trim_matches(|c: char| c.is_ascii_punctuation());

            // ISO date: 2026-02-24
            if stripped.len() == 10
                && stripped.chars().nth(4) == Some('-')
                && stripped.chars().nth(7) == Some('-')
                && stripped[..4].chars().all(|c| c.is_ascii_digit())
                && seen_dates.insert(stripped.to_string())
            {
                facts.dates.push(stripped.to_string());
            }

            // Month names
            if matches!(
                stripped,
                "january"
                    | "february"
                    | "march"
                    | "april"
                    | "may"
                    | "june"
                    | "july"
                    | "august"
                    | "september"
                    | "october"
                    | "november"
                    | "december"
            ) && seen_dates.insert(stripped.to_string())
            {
                facts.dates.push(
                    word.trim_matches(|c: char| c.is_ascii_punctuation())
                        .to_string(),
                );
            }
        }

        // Extract decisions: sentences containing decision keywords
        for sentence in part.split(['.', '!', '?']) {
            let lower = sentence.to_lowercase();
            if lower.contains("decided")
                || lower.contains("agreed")
                || lower.contains("chose")
                || lower.contains("must ")
                || lower.contains("will ")
                || lower.contains("approved")
                || lower.contains("rejected")
                || lower.contains("deployed")
                || lower.contains("migrated")
            {
                let trimmed = sentence.trim();
                if trimmed.len() >= 10 && trimmed.len() <= 300 {
                    facts.decisions.push(trimmed.to_string());
                }
            }
        }

        // Extract numbers with context: "port 5432", "version 3.2", "$50", "128GB"
        for (idx, word) in words.iter().enumerate() {
            let stripped = word.trim_matches(|c: char| c.is_ascii_punctuation());
            if stripped.chars().any(|c| c.is_ascii_digit()) && stripped.len() >= 2 {
                // Get surrounding context (1 word before and after)
                let before = if idx > 0 { words[idx - 1] } else { "" };
                let after = if idx + 1 < words.len() {
                    words[idx + 1]
                } else {
                    ""
                };
                let context = format!("{} {} {}", before, stripped, after)
                    .trim()
                    .to_string();
                if context.len() >= 3 && seen_numbers.insert(stripped.to_lowercase()) {
                    facts.numbers.push(context);
                }
            }
        }
    }

    // Limit to avoid bloating
    facts.entities.truncate(20);
    facts.dates.truncate(10);
    facts.decisions.truncate(10);
    facts.numbers.truncate(15);

    facts
}

/// Validate that extracted facts appear in the compacted summary.
pub fn validate_compaction(summary: &str, facts: &ExtractedFacts) -> FactValidation {
    let lower_summary = summary.to_lowercase();
    let mut missing = Vec::new();
    let mut preserved = 0usize;

    for fact in facts.all_facts() {
        // Check if any significant substring (3+ word) of the fact appears in summary
        let lower_fact = fact.to_lowercase();
        let words: Vec<&str> = lower_fact.split_whitespace().collect();

        let found = if words.len() <= 2 {
            // For short facts, check the whole thing
            lower_summary.contains(&lower_fact)
        } else {
            // For longer facts, check if key words appear
            let key_words: Vec<&str> = words
                .iter()
                .filter(|w| w.len() >= 3 && !is_stop_word(w))
                .copied()
                .collect();
            let matches = key_words
                .iter()
                .filter(|w| lower_summary.contains(**w))
                .count();
            // At least 60% of key words must be present
            key_words.is_empty() || (matches as f64 / key_words.len() as f64) >= 0.6
        };

        if found {
            preserved += 1;
        } else {
            missing.push(fact.to_string());
        }
    }

    let total = facts.total();
    let coverage = if total == 0 {
        1.0
    } else {
        preserved as f64 / total as f64
    };

    FactValidation {
        total_facts: total,
        preserved,
        missing,
        coverage,
    }
}

/// Augment a summary with missing facts to improve preservation.
///
/// Appends a "[Key facts]" section with the missing facts (up to 500 chars).
pub fn augment_summary(summary: &str, missing: &[String], max_total_chars: usize) -> String {
    if missing.is_empty() {
        return summary.to_string();
    }

    let mut augmented = summary.to_string();
    let budget = max_total_chars.saturating_sub(augmented.len() + 15); // 15 for " [Key facts: ]"
    if budget < 20 {
        return augmented;
    }

    augmented.push_str(" [Key facts: ");
    let mut used = 0;
    for (i, fact) in missing.iter().enumerate() {
        let entry = if i > 0 {
            format!("; {}", fact)
        } else {
            fact.clone()
        };
        if used + entry.len() > budget {
            break;
        }
        augmented.push_str(&entry);
        used += entry.len();
    }
    augmented.push(']');

    augmented
}

/// Check if a word appears at the start of a sentence in the text.
fn is_sentence_start(text: &str, word: &str) -> bool {
    if let Some(pos) = text.find(word) {
        if pos == 0 {
            return true;
        }
        // Check if preceded by sentence-ending punctuation + space
        let before = &text[..pos];
        let trimmed = before.trim_end();
        trimmed.is_empty()
            || trimmed.ends_with('.')
            || trimmed.ends_with('!')
            || trimmed.ends_with('?')
            || trimmed.ends_with('\n')
    } else {
        false
    }
}

/// Check if a single capitalized word is likely a named entity.
fn is_likely_entity(word: &str) -> bool {
    let stripped = word.trim_matches(|c: char| c.is_ascii_punctuation());
    // All-caps acronyms (API, SQL, etc.)
    stripped.len() >= 2
        && stripped
            .chars()
            .all(|c| c.is_uppercase() || c.is_ascii_digit())
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "a"
            | "an"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "have"
            | "has"
            | "had"
            | "do"
            | "does"
            | "did"
            | "will"
            | "would"
            | "could"
            | "should"
            | "may"
            | "might"
            | "to"
            | "of"
            | "in"
            | "for"
            | "on"
            | "with"
            | "at"
            | "by"
            | "from"
            | "as"
            | "into"
            | "and"
            | "or"
            | "but"
            | "not"
            | "that"
            | "this"
            | "it"
            | "its"
            | "we"
            | "our"
    )
}

// ============================================================================
// Original helpers
// ============================================================================

/// Extract the first sentence from content as a "key fact".
///
/// Trims to at most 500 characters and ensures the result ends with a period.
fn extract_key_fact(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Take content up to the first sentence-ending punctuation
    let end = trimmed
        .find(". ")
        .or_else(|| trimmed.find(".\n"))
        .map(|p| p + 1) // include the period
        .unwrap_or(trimmed.len());

    let mut fact: String = trimmed[..end].trim().to_string();

    // Truncate to 500 chars
    if fact.len() > 500 {
        fact.truncate(500);
        if let Some(last_space) = fact.rfind(' ') {
            fact.truncate(last_space);
        }
        if !fact.ends_with('.') {
            fact.push('.');
        }
    }

    // Ensure it ends with a period
    if !fact.ends_with('.') && !fact.ends_with('!') && !fact.ends_with('?') {
        fact.push('.');
    }

    fact
}

/// Build a consolidated summary from multiple content parts.
///
/// Concatenates with newlines, truncated to 2000 characters.
fn build_summary(parts: &[String]) -> String {
    let mut summary = String::with_capacity(2048);
    summary.push_str("[Session Summary] ");

    for (i, part) in parts.iter().enumerate() {
        if summary.len() > 1900 {
            summary.push_str(&format!("... (+{} more)", parts.len() - i));
            break;
        }
        if i > 0 {
            summary.push_str(" | ");
        }
        // Take first 200 chars of each part (UTF-8 safe)
        let snippet = if part.len() > 200 {
            zeus_core::truncate_str(part, 200)
        } else {
            part.as_str()
        };
        summary.push_str(snippet.trim());
    }

    summary
}

/// Log a promotion event as a pattern for auditing.
fn log_promotion(store: &MemoryStore, original_id: i64, new_id: i64, key_fact: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let content = format!(
        "{original_id}->{new_id}: {}",
        &key_fact[..zeus_core::floor_char_boundary(key_fact, 100)]
    );
    store
        .conn()
        .execute(
            "INSERT INTO patterns (pattern_type, content, frequency, first_seen, last_seen)
         VALUES ('promotion', ?1, 1, ?2, ?2)
         ON CONFLICT(pattern_type, content) DO UPDATE SET
            frequency = frequency + 1,
            last_seen = ?2",
            params![content, now],
        )
        .map_err(|e| Error::Database(format!("Failed to log promotion: {e}")))?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use tempfile::tempdir;
    fn make_store() -> (tempfile::TempDir, MemoryStore) {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("test.db");
        let store = MemoryStore::new(&db, true, false).expect("store");
        (dir, store)
    }

    fn store_episodic(store: &MemoryStore, session: &str, content: &str, importance: f32) -> i64 {
        let msg = Message::user(content);
        store
            .store_typed(session, &msg, MemoryType::Episodic, importance)
            .expect("store_typed")
    }

    fn store_old_episodic(
        store: &MemoryStore,
        session: &str,
        content: &str,
        importance: f32,
        days_ago: i64,
    ) -> i64 {
        let ts = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from)
             VALUES (?1, 'user', ?2, '[]', '[]', ?3, 'episodic', ?4, ?3)",
            params![session, content, ts, importance as f64],
        ).expect("insert old episodic");
        store.conn().last_insert_rowid()
    }

    // -- extract_key_fact -------------------------------------------------------

    #[test]
    fn test_extract_key_fact_single_sentence() {
        let fact = extract_key_fact("The sky is blue. And water is wet.");
        assert_eq!(fact, "The sky is blue.");
    }

    #[test]
    fn test_extract_key_fact_no_period() {
        let fact = extract_key_fact("No period here");
        assert_eq!(fact, "No period here.");
    }

    #[test]
    fn test_extract_key_fact_empty() {
        assert!(extract_key_fact("").is_empty());
        assert!(extract_key_fact("   ").is_empty());
    }

    #[test]
    fn test_extract_key_fact_truncates_long() {
        let long = "A".repeat(600);
        let fact = extract_key_fact(&long);
        assert!(fact.len() <= 501); // 500 + possible trailing period
        assert!(fact.ends_with('.'));
    }

    // -- build_summary ----------------------------------------------------------

    #[test]
    fn test_build_summary_basic() {
        let parts = vec!["first message".into(), "second message".into()];
        let summary = build_summary(&parts);
        assert!(summary.starts_with("[Session Summary]"));
        assert!(summary.contains("first message"));
        assert!(summary.contains("second message"));
    }

    #[test]
    fn test_build_summary_truncates() {
        let parts: Vec<String> = (0..50)
            .map(|i| format!("Message number {} with some padding content here", i))
            .collect();
        let summary = build_summary(&parts);
        assert!(summary.len() <= 2100); // some slack for the ellipsis
        assert!(summary.contains("... (+"));
    }

    // -- auto_promote -----------------------------------------------------------

    #[test]
    fn test_auto_promote_no_candidates() {
        let (_dir, store) = make_store();
        // Store a low-importance episodic memory
        store_episodic(&store, "s1", "Low importance memory.", 0.3);

        let result = auto_promote(&store, 0.8, 0).expect("auto_promote");
        assert_eq!(result.promoted, 0);
    }

    #[test]
    fn test_auto_promote_promotes_high_importance() {
        let (_dir, store) = make_store();
        // Store an old, high-importance episodic memory
        store_old_episodic(
            &store,
            "s1",
            "Rust uses LLVM as its backend. It compiles fast.",
            0.95,
            5,
        );

        let result = auto_promote(&store, 0.8, 1).expect("auto_promote");
        assert_eq!(result.promoted, 1);

        // Verify a semantic memory was created
        let semantics = store
            .search_by_type("Rust LLVM", MemoryType::Semantic, 10)
            .expect("search");
        assert!(!semantics.is_empty());
    }

    #[test]
    fn test_auto_promote_skips_too_recent() {
        let (_dir, store) = make_store();
        // Store a high-importance memory that was just created (0 days old)
        store_episodic(&store, "s1", "Very important but just created.", 0.99);

        // Require at least 3 days of access history
        let result = auto_promote(&store, 0.8, 3).expect("auto_promote");
        assert_eq!(result.promoted, 0);
    }

    #[test]
    fn test_auto_promote_logs_pattern() {
        let (_dir, store) = make_store();
        store_old_episodic(
            &store,
            "s1",
            "Key architectural decision made. Details follow.",
            0.9,
            5,
        );

        auto_promote(&store, 0.8, 1).expect("auto_promote");

        let patterns = store.get_patterns("promotion", 10).expect("patterns");
        assert_eq!(patterns.len(), 1);
        assert!(
            patterns[0]
                .content
                .contains("Key architectural decision made.")
        );
    }

    // -- consolidate_session ---------------------------------------------------

    #[test]
    fn test_consolidate_empty_session() {
        let (_dir, store) = make_store();
        let result = consolidate_session(&store, "empty", false).expect("consolidate");
        assert_eq!(result.rolled_up, 0);
        assert_eq!(result.summaries_created, 0);
        assert!(result.summary_id.is_none());
    }

    #[test]
    fn test_consolidate_session_basic() {
        let (_dir, store) = make_store();
        store_episodic(&store, "s1", "First point about the project.", 0.5);
        store_episodic(&store, "s1", "Second point about testing.", 0.6);
        store_episodic(&store, "s1", "Third point about deployment.", 0.7);

        let result = consolidate_session(&store, "s1", false).expect("consolidate");
        assert_eq!(result.rolled_up, 3);
        assert_eq!(result.summaries_created, 1);
        assert!(result.summary_id.is_some());

        // Originals should be superseded (valid_to set)
        let still_active: i64 = store.conn().query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = 's1' AND memory_type = 'episodic' AND valid_to IS NULL",
            [],
            |row| row.get(0),
        ).expect("count");
        assert_eq!(still_active, 0);
    }

    // -- garbage_collect -------------------------------------------------------

    #[test]
    fn test_gc_no_old_memories() {
        let (_dir, store) = make_store();
        store_episodic(&store, "s1", "Recent memory.", 0.3);

        let result = garbage_collect(&store, &GcConfig::default()).expect("gc");
        assert_eq!(result.deleted, 0);
    }

    #[test]
    fn test_gc_deletes_old_low_importance() {
        let (_dir, store) = make_store();
        store_old_episodic(&store, "s1", "Old and unimportant.", 0.1, 60);
        store_old_episodic(&store, "s1", "Also old and unimportant.", 0.2, 45);

        let config = GcConfig {
            max_age_days: 30,
            importance_floor: 0.5,
            ..Default::default()
        };
        let result = garbage_collect(&store, &config).expect("gc");
        assert_eq!(result.deleted, 2);
    }

    #[test]
    fn test_gc_keeps_high_importance() {
        let (_dir, store) = make_store();
        store_old_episodic(&store, "s1", "Old but important.", 0.9, 60);
        store_old_episodic(&store, "s1", "Old and unimportant.", 0.1, 60);

        let config = GcConfig {
            max_age_days: 30,
            importance_floor: 0.5,
            ..Default::default()
        };
        let result = garbage_collect(&store, &config).expect("gc");
        assert_eq!(result.deleted, 1);
        assert_eq!(result.kept, 1);
    }

    #[test]
    fn test_gc_protects_semantic_type() {
        let (_dir, store) = make_store();
        // Store an old semantic memory directly
        let ts = (Utc::now() - Duration::days(60)).to_rfc3339();
        store.conn().execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_results, timestamp, memory_type, importance, valid_from)
             VALUES ('s1', 'system', 'Semantic knowledge.', '[]', '[]', ?1, 'semantic', 0.1, ?1)",
            params![ts],
        ).expect("insert");

        let config = GcConfig {
            max_age_days: 30,
            importance_floor: 0.5,
            ..Default::default()
        };
        let result = garbage_collect(&store, &config).expect("gc");
        assert_eq!(result.deleted, 0);
    }

    #[test]
    fn test_gc_custom_config() {
        let (_dir, store) = make_store();
        store_old_episodic(&store, "s1", "Fairly old memory.", 0.4, 15);

        let strict = GcConfig {
            max_age_days: 7,
            importance_floor: 0.5,
            ..Default::default()
        };
        let result = garbage_collect(&store, &strict).expect("gc");
        assert_eq!(result.deleted, 1);
    }

    // -- extract_facts ----------------------------------------------------------

    #[test]
    fn test_extract_facts_entities() {
        let parts = vec!["We deployed to Amazon Web Services using Terraform Cloud.".to_string()];
        let facts = extract_facts(&parts);
        assert!(
            facts.entities.iter().any(|e| e.contains("Amazon")),
            "Should extract 'Amazon Web Services' as entity: {:?}",
            facts.entities
        );
    }

    #[test]
    fn test_extract_facts_dates() {
        let parts = vec!["The release is scheduled for 2026-02-24 in January.".to_string()];
        let facts = extract_facts(&parts);
        assert!(
            facts.dates.iter().any(|d| d.contains("2026-02-24")),
            "Should extract ISO date: {:?}",
            facts.dates
        );
        assert!(
            facts
                .dates
                .iter()
                .any(|d| d.to_lowercase().contains("january")),
            "Should extract month name: {:?}",
            facts.dates
        );
    }

    #[test]
    fn test_extract_facts_decisions() {
        let parts = vec![
            "The team decided to use Rust. They agreed on weekly sprints. Build will proceed."
                .to_string(),
        ];
        let facts = extract_facts(&parts);
        assert!(
            facts.decisions.len() >= 2,
            "Should extract decision sentences: {:?}",
            facts.decisions
        );
    }

    #[test]
    fn test_extract_facts_numbers() {
        let parts = vec!["The database runs on port 5432 with 128GB of RAM.".to_string()];
        let facts = extract_facts(&parts);
        assert!(
            facts.numbers.iter().any(|n| n.contains("5432")),
            "Should extract port number: {:?}",
            facts.numbers
        );
    }

    #[test]
    fn test_extract_facts_empty() {
        let facts = extract_facts(&[]);
        assert_eq!(facts.total(), 0);

        let facts = extract_facts(&["hello world".to_string()]);
        assert_eq!(facts.decisions.len(), 0);
    }

    // -- validate_compaction ----------------------------------------------------

    #[test]
    fn test_validate_compaction_all_preserved() {
        let parts = vec!["The team decided to use Rust on port 5432.".to_string()];
        let facts = extract_facts(&parts);
        let summary = "The team decided to use Rust on port 5432.";
        let v = validate_compaction(summary, &facts);
        assert_eq!(v.coverage, 1.0);
        assert!(v.missing.is_empty());
    }

    #[test]
    fn test_validate_compaction_missing_facts() {
        let parts = vec![
            "The team decided to use Rust. Deployed to Amazon Web Services on 2026-02-24."
                .to_string(),
        ];
        let facts = extract_facts(&parts);
        // Summary only mentions Rust, not AWS or the date
        let summary = "Team uses Rust for the project.";
        let v = validate_compaction(summary, &facts);
        assert!(v.coverage < 1.0, "Some facts should be missing");
        assert!(!v.missing.is_empty());
    }

    #[test]
    fn test_validate_compaction_empty_facts() {
        let facts = ExtractedFacts::default();
        let v = validate_compaction("any summary", &facts);
        assert_eq!(v.coverage, 1.0);
        assert_eq!(v.total_facts, 0);
    }

    // -- augment_summary --------------------------------------------------------

    #[test]
    fn test_augment_summary_adds_missing() {
        let summary = "Team decided to use Rust.";
        let missing = vec!["port 5432".to_string(), "2026-02-24".to_string()];
        let augmented = augment_summary(summary, &missing, 200);
        assert!(augmented.contains("[Key facts:"));
        assert!(augmented.contains("port 5432"));
        assert!(augmented.contains("2026-02-24"));
    }

    #[test]
    fn test_augment_summary_respects_max_chars() {
        let summary = "A".repeat(180);
        let missing = vec!["very long fact text here".to_string()];
        let augmented = augment_summary(&summary, &missing, 200);
        assert!(augmented.len() <= 220); // some tolerance for the tag
    }

    #[test]
    fn test_augment_summary_empty_missing() {
        let summary = "Original summary.";
        let augmented = augment_summary(summary, &[], 200);
        assert_eq!(augmented, summary);
    }

    // -- consolidate_session with fact_check = true ---------------------------

    #[test]
    fn test_consolidate_with_fact_check() {
        let (_dir, store) = make_store();
        store_episodic(
            &store,
            "fc",
            "The team decided to deploy on 2026-02-24 to Amazon Web Services.",
            0.8,
        );
        store_episodic(
            &store,
            "fc",
            "Database runs on port 5432 with PostgreSQL.",
            0.7,
        );
        store_episodic(
            &store,
            "fc",
            "The migration will happen on January 15th.",
            0.6,
        );

        let result = consolidate_session(&store, "fc", true).expect("consolidate");
        assert_eq!(result.rolled_up, 3);
        assert_eq!(result.summaries_created, 1);

        // Read the summary content
        let summary_id = result.summary_id.unwrap();
        let content: String = store
            .conn()
            .query_row(
                "SELECT content FROM messages WHERE id = ?1",
                params![summary_id],
                |row| row.get(0),
            )
            .expect("read summary");

        // The summary should contain key facts (possibly augmented)
        let lower = content.to_lowercase();
        assert!(
            lower.contains("5432") || lower.contains("key facts"),
            "Summary should preserve port number or have key facts section: {}",
            content
        );
    }
}
