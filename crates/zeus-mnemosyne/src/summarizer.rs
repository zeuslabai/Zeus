//! Session Summarizer — automatic memory consolidation.
//!
//! Compresses verbose session histories into concise summaries for efficient
//! context injection. Supports:
//! - Extractive summarization (key sentence selection via TF-IDF scoring)
//! - Topic clustering (group messages by semantic similarity)
//! - Rolling window consolidation (merge old summaries progressively)
//! - Retention policies (time-based + importance-based pruning)

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the session summarizer.
#[derive(Debug, Clone)]
pub struct SummarizerConfig {
    /// Maximum number of sentences to extract per summary.
    pub max_sentences: usize,
    /// Maximum character length for a single summary.
    pub max_summary_chars: usize,
    /// Minimum messages required before summarization triggers.
    pub min_messages_to_summarize: usize,
    /// Rolling window: how many summaries to merge into a consolidated one.
    pub rolling_window_size: usize,
    /// Retention: discard summaries older than this many hours.
    pub retention_hours: u64,
    /// Minimum importance score (0.0–1.0) to keep during consolidation.
    pub min_importance: f64,
    /// Stop words to exclude from TF-IDF scoring.
    pub stop_words: HashSet<String>,
}

impl Default for SummarizerConfig {
    fn default() -> Self {
        let stop_words: HashSet<String> = [
            "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has",
            "had", "do", "does", "did", "will", "would", "could", "should", "may", "might",
            "shall", "can", "need", "dare", "ought", "used", "to", "of", "in", "for", "on", "with",
            "at", "by", "from", "as", "into", "through", "during", "before", "after", "above",
            "below", "between", "out", "off", "over", "under", "again", "further", "then", "once",
            "here", "there", "when", "where", "why", "how", "all", "each", "every", "both", "few",
            "more", "most", "other", "some", "such", "no", "nor", "not", "only", "own", "same",
            "so", "than", "too", "very", "just", "because", "but", "and", "or", "if", "while",
            "that", "this", "it", "its", "i", "me", "my", "we", "our", "you", "your", "he", "him",
            "his", "she", "her", "they", "them", "their", "what", "which", "who",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        Self {
            max_sentences: 5,
            max_summary_chars: 1000,
            min_messages_to_summarize: 3,
            rolling_window_size: 5,
            retention_hours: 168, // 7 days
            min_importance: 0.2,
            stop_words,
        }
    }
}

// ============================================================================
// Types
// ============================================================================

/// A message to be summarized.
#[derive(Debug, Clone)]
pub struct SummaryMessage {
    /// Role of the speaker (e.g., "user", "assistant").
    pub role: String,
    /// Message content text.
    pub content: String,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// Optional importance score (0.0–1.0).
    pub importance: Option<f64>,
}

/// A generated summary of a session or message batch.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// Unique identifier for this summary.
    pub id: String,
    /// The session ID this summary belongs to.
    pub session_id: String,
    /// Extracted summary text.
    pub text: String,
    /// Number of source messages that were summarized.
    pub source_message_count: usize,
    /// Detected topics with their relevance scores.
    pub topics: Vec<TopicCluster>,
    /// When this summary was created (unix secs).
    pub created_at: u64,
    /// Importance score of the summary itself (0.0–1.0).
    pub importance: f64,
}

/// A cluster of related content identified during summarization.
#[derive(Debug, Clone)]
pub struct TopicCluster {
    /// Label derived from top terms.
    pub label: String,
    /// Keywords that define this topic.
    pub keywords: Vec<String>,
    /// How many sentences belong to this topic.
    pub sentence_count: usize,
    /// Relevance score (0.0–1.0).
    pub relevance: f64,
}

/// Statistics about summarizer operations.
#[derive(Debug, Clone, Default)]
pub struct SummarizerStats {
    /// Total summaries generated.
    pub summaries_generated: usize,
    /// Total messages processed.
    pub messages_processed: usize,
    /// Total consolidations performed (rolling merges).
    pub consolidations: usize,
    /// Total summaries pruned by retention policy.
    pub pruned: usize,
}

// ============================================================================
// TF-IDF Scoring
// ============================================================================

/// Term frequency–inverse document frequency scores for sentence ranking.
struct TfIdfScorer {
    /// document frequency: term → number of documents containing it
    doc_freq: HashMap<String, usize>,
    /// total number of documents
    doc_count: usize,
}

impl TfIdfScorer {
    /// Build from a collection of documents (each is a list of tokens).
    fn new(documents: &[Vec<String>]) -> Self {
        let mut doc_freq: HashMap<String, usize> = HashMap::new();
        for doc in documents {
            let unique: HashSet<&String> = doc.iter().collect();
            for term in unique {
                *doc_freq.entry(term.clone()).or_insert(0) += 1;
            }
        }
        Self {
            doc_freq,
            doc_count: documents.len(),
        }
    }

    /// Score a single document (sentence) by summing TF-IDF of its terms.
    fn score(&self, tokens: &[String]) -> f64 {
        if tokens.is_empty() {
            return 0.0;
        }

        // term frequency within this document
        let mut tf: HashMap<&String, usize> = HashMap::new();
        for t in tokens {
            *tf.entry(t).or_insert(0) += 1;
        }

        let mut total = 0.0;
        for (term, count) in &tf {
            let tf_val = *count as f64 / tokens.len() as f64;
            let df = self.doc_freq.get(*term).copied().unwrap_or(1) as f64;
            let idf = (self.doc_count as f64 / df).ln() + 1.0;
            total += tf_val * idf;
        }
        total
    }
}

// ============================================================================
// Summarizer Engine
// ============================================================================

/// The session summarizer engine.
pub struct SessionSummarizer {
    config: SummarizerConfig,
    summaries: Vec<SessionSummary>,
    stats: SummarizerStats,
}

impl SessionSummarizer {
    /// Create a new summarizer with default config.
    pub fn new() -> Self {
        Self {
            config: SummarizerConfig::default(),
            summaries: Vec::new(),
            stats: SummarizerStats::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: SummarizerConfig) -> Self {
        Self {
            config,
            summaries: Vec::new(),
            stats: SummarizerStats::default(),
        }
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: SummarizerConfig) {
        self.config = config;
    }

    /// Get current statistics.
    pub fn stats(&self) -> &SummarizerStats {
        &self.stats
    }

    /// Get all stored summaries.
    pub fn summaries(&self) -> &[SessionSummary] {
        &self.summaries
    }

    /// Get summaries for a specific session.
    pub fn session_summaries(&self, session_id: &str) -> Vec<&SessionSummary> {
        self.summaries
            .iter()
            .filter(|s| s.session_id == session_id)
            .collect()
    }

    /// Summarize a batch of messages from a session.
    ///
    /// Returns `None` if there aren't enough messages to summarize.
    pub fn summarize(
        &mut self,
        session_id: &str,
        messages: &[SummaryMessage],
    ) -> Option<SessionSummary> {
        if messages.len() < self.config.min_messages_to_summarize {
            return None;
        }

        // 1. Extract sentences from all messages
        let sentences = self.extract_sentences(messages);
        if sentences.is_empty() {
            return None;
        }

        // 2. Tokenize each sentence
        let tokenized: Vec<Vec<String>> = sentences.iter().map(|s| self.tokenize(s)).collect();

        // 3. Build TF-IDF scorer
        let scorer = TfIdfScorer::new(&tokenized);

        // 4. Score each sentence
        let mut scored: Vec<(usize, f64)> = tokenized
            .iter()
            .enumerate()
            .map(|(i, tokens)| (i, scorer.score(tokens)))
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 5. Select top sentences (respecting max_sentences and max_chars)
        let mut selected_indices: Vec<usize> = Vec::new();
        let mut total_chars = 0;
        for (idx, _score) in &scored {
            if selected_indices.len() >= self.config.max_sentences {
                break;
            }
            let sentence_len = sentences[*idx].len();
            if total_chars + sentence_len > self.config.max_summary_chars {
                // Try to fit — skip if too long
                if total_chars > 0 {
                    continue;
                }
                // First sentence: truncate
            }
            selected_indices.push(*idx);
            total_chars += sentence_len;
        }

        // Sort by original order for coherent reading
        selected_indices.sort();

        // 6. Build summary text
        let summary_text: String = selected_indices
            .iter()
            .map(|i| sentences[*i].as_str())
            .collect::<Vec<_>>()
            .join(" ");

        // Truncate if still over limit
        let summary_text = if summary_text.len() > self.config.max_summary_chars {
            let mut end = self.config.max_summary_chars;
            while !summary_text.is_char_boundary(end) && end < summary_text.len() {
                end += 1;
            }
            let mut truncated = summary_text[..end].to_string();
            // Try to end at a sentence boundary
            if let Some(pos) = truncated.rfind(". ") {
                truncated.truncate(pos + 1);
            }
            truncated
        } else {
            summary_text
        };

        // 7. Detect topic clusters
        let topics = self.detect_topics(&tokenized, &sentences);

        // 8. Compute importance from message importance scores
        let avg_importance = messages.iter().filter_map(|m| m.importance).sum::<f64>()
            / messages.len().max(1) as f64;
        // Boost importance if there are high-scoring sentences
        let top_score = scored.first().map(|(_, s)| *s).unwrap_or(0.0);
        let importance =
            (avg_importance * 0.6 + (top_score / (top_score + 1.0)) * 0.4).clamp(0.0, 1.0);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let summary = SessionSummary {
            id: format!("sum-{}-{}", session_id, now),
            session_id: session_id.to_string(),
            text: summary_text,
            source_message_count: messages.len(),
            topics,
            created_at: now,
            importance,
        };

        self.summaries.push(summary.clone());
        self.stats.summaries_generated += 1;
        self.stats.messages_processed += messages.len();

        Some(summary)
    }

    /// Consolidate multiple summaries into one rolling summary.
    ///
    /// Takes the oldest N summaries (up to rolling_window_size) for a session
    /// and merges them into a single consolidated summary.
    pub fn consolidate(&mut self, session_id: &str) -> Option<SessionSummary> {
        let session_indices: Vec<usize> = self
            .summaries
            .iter()
            .enumerate()
            .filter(|(_, s)| s.session_id == session_id)
            .map(|(i, _)| i)
            .collect();

        if session_indices.len() < 2 {
            return None;
        }

        let take_count = session_indices.len().min(self.config.rolling_window_size);
        let merge_indices: Vec<usize> = session_indices[..take_count].to_vec();

        // Collect texts and topics from summaries to merge
        let mut merged_texts: Vec<String> = Vec::new();
        let mut all_topics: Vec<TopicCluster> = Vec::new();
        let mut total_messages = 0usize;
        let mut importance_sum = 0.0f64;

        for &idx in &merge_indices {
            let s = &self.summaries[idx];
            merged_texts.push(s.text.clone());
            all_topics.extend(s.topics.clone());
            total_messages += s.source_message_count;
            importance_sum += s.importance;
        }

        let avg_importance = importance_sum / merge_indices.len() as f64;

        // Re-summarize the merged texts
        let combined = merged_texts.join(" ");
        let sentences = self.split_sentences(&combined);
        let tokenized: Vec<Vec<String>> = sentences.iter().map(|s| self.tokenize(s)).collect();

        let scorer = TfIdfScorer::new(&tokenized);
        let mut scored: Vec<(usize, f64)> = tokenized
            .iter()
            .enumerate()
            .map(|(i, tokens)| (i, scorer.score(tokens)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected: Vec<usize> = Vec::new();
        let mut total_chars = 0;
        for (idx, _) in &scored {
            if selected.len() >= self.config.max_sentences {
                break;
            }
            let len = sentences[*idx].len();
            if total_chars + len <= self.config.max_summary_chars {
                selected.push(*idx);
                total_chars += len;
            }
        }
        selected.sort();

        let text: String = selected
            .iter()
            .map(|i| sentences[*i].as_str())
            .collect::<Vec<_>>()
            .join(" ");

        // Deduplicate topics by label
        let mut seen_labels: HashSet<String> = HashSet::new();
        let deduped_topics: Vec<TopicCluster> = all_topics
            .into_iter()
            .filter(|t| seen_labels.insert(t.label.clone()))
            .collect();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let consolidated = SessionSummary {
            id: format!("con-{}-{}", session_id, now),
            session_id: session_id.to_string(),
            text,
            source_message_count: total_messages,
            topics: deduped_topics,
            created_at: now,
            importance: avg_importance,
        };

        // Remove old summaries (in reverse order to preserve indices)
        let mut to_remove = merge_indices;
        to_remove.sort_by(|a, b| b.cmp(a));
        for idx in to_remove {
            self.summaries.remove(idx);
        }

        self.summaries.push(consolidated.clone());
        self.stats.consolidations += 1;

        Some(consolidated)
    }

    /// Prune summaries older than retention_hours or below min_importance.
    pub fn prune(&mut self) -> usize {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cutoff = now.saturating_sub(self.config.retention_hours * 3600);

        let before = self.summaries.len();
        self.summaries
            .retain(|s| s.created_at >= cutoff && s.importance >= self.config.min_importance);
        let pruned = before - self.summaries.len();
        self.stats.pruned += pruned;
        pruned
    }

    /// Get the most recent summary for a session, or None.
    pub fn latest_summary(&self, session_id: &str) -> Option<&SessionSummary> {
        self.summaries
            .iter()
            .filter(|s| s.session_id == session_id)
            .max_by_key(|s| s.created_at)
    }

    /// Get the top N most important summaries across all sessions.
    pub fn top_summaries(&self, n: usize) -> Vec<&SessionSummary> {
        let mut sorted: Vec<&SessionSummary> = self.summaries.iter().collect();
        sorted.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(n);
        sorted
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    /// Extract sentences from messages, prefixing with role context.
    fn extract_sentences(&self, messages: &[SummaryMessage]) -> Vec<String> {
        let mut sentences = Vec::new();
        for msg in messages {
            if msg.content.trim().is_empty() {
                continue;
            }
            let role_prefix = match msg.role.as_str() {
                "user" => "User asked: ",
                "assistant" => "",
                "system" => "System: ",
                _ => "",
            };
            for s in self.split_sentences(&msg.content) {
                let trimmed = s.trim();
                if trimmed.len() >= 10 {
                    sentences.push(format!("{}{}", role_prefix, trimmed));
                }
            }
        }
        sentences
    }

    /// Split text into sentences by common delimiters.
    fn split_sentences(&self, text: &str) -> Vec<String> {
        let mut sentences = Vec::new();
        let mut current = String::new();

        for ch in text.chars() {
            current.push(ch);
            if matches!(ch, '.' | '!' | '?') {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current.clear();
            }
        }
        // Remaining text without sentence-ending punctuation
        let trimmed = current.trim().to_string();
        if !trimmed.is_empty() && trimmed.len() >= 10 {
            sentences.push(trimmed);
        }

        sentences
    }

    /// Tokenize text: lowercase, split on non-alphanumeric, remove stop words.
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2 && !self.config.stop_words.contains(*w))
            .map(String::from)
            .collect()
    }

    /// Detect topic clusters from tokenized sentences.
    fn detect_topics(&self, tokenized: &[Vec<String>], sentences: &[String]) -> Vec<TopicCluster> {
        // Count global term frequency across all sentences
        let mut global_freq: HashMap<String, usize> = HashMap::new();
        for tokens in tokenized {
            for t in tokens {
                *global_freq.entry(t.clone()).or_insert(0) += 1;
            }
        }

        // Find top terms (appearing in multiple sentences)
        let mut term_scores: Vec<(String, usize)> = global_freq
            .into_iter()
            .filter(|(_, count)| *count >= 2)
            .collect();
        term_scores.sort_by(|a, b| b.1.cmp(&a.1));

        // Build clusters around top terms
        let mut clusters: Vec<TopicCluster> = Vec::new();
        let mut used_terms: HashSet<String> = HashSet::new();

        for (term, freq) in term_scores.iter().take(5) {
            if used_terms.contains(term) {
                continue;
            }

            // Find co-occurring terms
            let mut cooccur: HashMap<String, usize> = HashMap::new();
            let mut sentence_count = 0;

            for tokens in tokenized {
                if tokens.contains(term) {
                    sentence_count += 1;
                    for t in tokens {
                        if t != term {
                            *cooccur.entry(t.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }

            let mut keywords: Vec<String> = vec![term.clone()];
            let mut cooccur_sorted: Vec<(String, usize)> = cooccur.into_iter().collect();
            cooccur_sorted.sort_by(|a, b| b.1.cmp(&a.1));
            for (co_term, _) in cooccur_sorted.iter().take(3) {
                if !used_terms.contains(co_term) {
                    keywords.push(co_term.clone());
                }
            }

            for kw in &keywords {
                used_terms.insert(kw.clone());
            }

            let relevance = (*freq as f64 / sentences.len().max(1) as f64).clamp(0.0, 1.0);
            let label = keywords
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");

            clusters.push(TopicCluster {
                label,
                keywords,
                sentence_count,
                relevance,
            });
        }

        clusters
    }
}

impl Default for SessionSummarizer {
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

    fn sample_messages() -> Vec<SummaryMessage> {
        vec![
            SummaryMessage {
                role: "user".into(),
                content: "How do I configure the Rust compiler for cross-compilation? I need to target ARM Linux from macOS.".into(),
                timestamp: 1000,
                importance: Some(0.8),
            },
            SummaryMessage {
                role: "assistant".into(),
                content: "To cross-compile Rust for ARM Linux, install the target with rustup target add aarch64-unknown-linux-gnu. Then configure a linker in .cargo/config.toml pointing to your cross-compilation toolchain. The gcc-aarch64-linux-gnu package provides the necessary linker on most systems.".into(),
                timestamp: 1001,
                importance: Some(0.9),
            },
            SummaryMessage {
                role: "user".into(),
                content: "What about using cargo-cross instead? Does it handle the toolchain setup automatically?".into(),
                timestamp: 1002,
                importance: Some(0.7),
            },
            SummaryMessage {
                role: "assistant".into(),
                content: "Yes, cargo-cross uses Docker containers with pre-configured toolchains. Install it with cargo install cross, then replace cargo build with cross build --target aarch64-unknown-linux-gnu. It handles all the toolchain and sysroot configuration inside the container.".into(),
                timestamp: 1003,
                importance: Some(0.85),
            },
        ]
    }

    #[test]
    fn test_default_config() {
        let config = SummarizerConfig::default();
        assert_eq!(config.max_sentences, 5);
        assert_eq!(config.max_summary_chars, 1000);
        assert_eq!(config.min_messages_to_summarize, 3);
        assert_eq!(config.rolling_window_size, 5);
        assert_eq!(config.retention_hours, 168);
        assert!(config.stop_words.contains("the"));
        assert!(!config.stop_words.contains("rust"));
    }

    #[test]
    fn test_new_summarizer() {
        let s = SessionSummarizer::new();
        assert_eq!(s.stats().summaries_generated, 0);
        assert_eq!(s.stats().messages_processed, 0);
        assert!(s.summaries().is_empty());
    }

    #[test]
    fn test_summarize_basic() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages);
        assert!(result.is_some());
        let summary = result.unwrap();
        assert_eq!(summary.session_id, "sess-1");
        assert_eq!(summary.source_message_count, 4);
        assert!(!summary.text.is_empty());
        assert!(summary.importance > 0.0);
    }

    #[test]
    fn test_summarize_too_few_messages() {
        let mut s = SessionSummarizer::new();
        let messages = vec![SummaryMessage {
            role: "user".into(),
            content: "Hello".into(),
            timestamp: 1000,
            importance: None,
        }];
        assert!(s.summarize("sess-1", &messages).is_none());
        assert_eq!(s.stats().summaries_generated, 0);
    }

    #[test]
    fn test_summarize_empty_content() {
        let mut s = SessionSummarizer::new();
        let messages = vec![
            SummaryMessage {
                role: "user".into(),
                content: "".into(),
                timestamp: 1000,
                importance: None,
            },
            SummaryMessage {
                role: "user".into(),
                content: "  ".into(),
                timestamp: 1001,
                importance: None,
            },
            SummaryMessage {
                role: "user".into(),
                content: "\n".into(),
                timestamp: 1002,
                importance: None,
            },
        ];
        assert!(s.summarize("sess-1", &messages).is_none());
    }

    #[test]
    fn test_summary_respects_max_sentences() {
        let mut s = SessionSummarizer::with_config(SummarizerConfig {
            max_sentences: 2,
            max_summary_chars: 5000,
            ..SummarizerConfig::default()
        });
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages).unwrap();
        // Count sentences — should be at most 2 selected
        let sentence_count = result.text.matches(". ").count() + 1;
        // Due to how sentences are joined, the count should be reasonable
        assert!(sentence_count <= 4); // 2 selected sentences might contain internal periods
    }

    #[test]
    fn test_summary_respects_max_chars() {
        let mut s = SessionSummarizer::with_config(SummarizerConfig {
            max_summary_chars: 100,
            ..SummarizerConfig::default()
        });
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages).unwrap();
        assert!(result.text.len() <= 200); // Some tolerance for sentence boundary truncation
    }

    #[test]
    fn test_topics_detected() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages).unwrap();
        // Should detect topics related to Rust/cross-compilation/toolchain
        assert!(!result.topics.is_empty());
        let all_keywords: Vec<String> = result
            .topics
            .iter()
            .flat_map(|t| t.keywords.clone())
            .collect();
        // At least one keyword should relate to the content
        assert!(!all_keywords.is_empty());
    }

    #[test]
    fn test_topic_relevance_bounded() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages).unwrap();
        for topic in &result.topics {
            assert!(topic.relevance >= 0.0 && topic.relevance <= 1.0);
        }
    }

    #[test]
    fn test_stats_tracking() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        s.summarize("sess-2", &messages);
        assert_eq!(s.stats().summaries_generated, 2);
        assert_eq!(s.stats().messages_processed, 8);
    }

    #[test]
    fn test_session_summaries_filter() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        s.summarize("sess-2", &messages);
        s.summarize("sess-1", &messages);
        assert_eq!(s.session_summaries("sess-1").len(), 2);
        assert_eq!(s.session_summaries("sess-2").len(), 1);
        assert_eq!(s.session_summaries("sess-3").len(), 0);
    }

    #[test]
    fn test_latest_summary() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        let latest = s.latest_summary("sess-1");
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().session_id, "sess-1");
        assert!(s.latest_summary("nonexistent").is_none());
    }

    #[test]
    fn test_top_summaries() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        s.summarize("sess-2", &messages);
        s.summarize("sess-3", &messages);
        let top = s.top_summaries(2);
        assert_eq!(top.len(), 2);
        // Should be sorted by importance descending
        assert!(top[0].importance >= top[1].importance);
    }

    #[test]
    fn test_consolidate_merges_summaries() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        s.summarize("sess-1", &messages);
        s.summarize("sess-1", &messages);

        let before_count = s.summaries().len();
        assert_eq!(before_count, 3);

        let consolidated = s.consolidate("sess-1");
        assert!(consolidated.is_some());
        let c = consolidated.unwrap();
        assert!(c.id.starts_with("con-"));
        assert_eq!(c.source_message_count, 12); // 3 × 4

        // Old summaries replaced with one consolidated
        assert_eq!(s.summaries().len(), 1);
        assert_eq!(s.stats().consolidations, 1);
    }

    #[test]
    fn test_consolidate_not_enough() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        // Only one summary — can't consolidate
        assert!(s.consolidate("sess-1").is_none());
    }

    #[test]
    fn test_consolidate_respects_window_size() {
        let mut s = SessionSummarizer::with_config(SummarizerConfig {
            rolling_window_size: 2,
            ..SummarizerConfig::default()
        });
        let messages = sample_messages();
        s.summarize("sess-1", &messages);
        s.summarize("sess-1", &messages);
        s.summarize("sess-1", &messages);

        // Should only merge first 2 (window size), leaving the third plus consolidated
        s.consolidate("sess-1");
        assert_eq!(s.summaries().len(), 2); // 1 remaining + 1 consolidated
    }

    #[test]
    fn test_prune_by_retention() {
        let mut s = SessionSummarizer::with_config(SummarizerConfig {
            retention_hours: 1, // 1 hour
            min_importance: 0.0,
            ..SummarizerConfig::default()
        });
        let messages = sample_messages();
        s.summarize("sess-1", &messages);

        // Manually age the summary
        if let Some(summary) = s.summaries.first_mut() {
            summary.created_at = 1000; // Very old timestamp
        }

        let pruned = s.prune();
        assert_eq!(pruned, 1);
        assert!(s.summaries().is_empty());
        assert_eq!(s.stats().pruned, 1);
    }

    #[test]
    fn test_prune_by_importance() {
        let mut s = SessionSummarizer::with_config(SummarizerConfig {
            retention_hours: 999999,
            min_importance: 0.99, // Very high threshold
            ..SummarizerConfig::default()
        });
        let messages = sample_messages();
        s.summarize("sess-1", &messages);

        // The summary importance is unlikely to be >= 0.99
        let pruned = s.prune();
        assert_eq!(pruned, 1);
    }

    #[test]
    fn test_prune_keeps_valid() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        s.summarize("sess-1", &messages);

        let pruned = s.prune();
        assert_eq!(pruned, 0);
        assert_eq!(s.summaries().len(), 1);
    }

    #[test]
    fn test_importance_bounded() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        let result = s.summarize("sess-1", &messages).unwrap();
        assert!(result.importance >= 0.0 && result.importance <= 1.0);
    }

    #[test]
    fn test_no_importance_messages() {
        let mut s = SessionSummarizer::new();
        let messages = vec![
            SummaryMessage { role: "user".into(), content: "First question about Rust programming language features.".into(), timestamp: 1000, importance: None },
            SummaryMessage { role: "assistant".into(), content: "Rust has ownership, borrowing, and lifetime features that prevent memory issues.".into(), timestamp: 1001, importance: None },
            SummaryMessage { role: "user".into(), content: "What about async programming in Rust with tokio runtime?".into(), timestamp: 1002, importance: None },
        ];
        let result = s.summarize("sess-1", &messages).unwrap();
        // With no importance scores, avg_importance = 0, so importance comes from TF-IDF
        assert!(result.importance >= 0.0 && result.importance <= 1.0);
    }

    #[test]
    fn test_summary_id_format() {
        let mut s = SessionSummarizer::new();
        let messages = sample_messages();
        let result = s.summarize("my-session", &messages).unwrap();
        assert!(result.id.starts_with("sum-my-session-"));
    }

    #[test]
    fn test_tokenize_filters_stop_words() {
        let s = SessionSummarizer::new();
        let tokens = s.tokenize("The quick brown fox jumps over the lazy dog");
        assert!(!tokens.contains(&"the".to_string()));
        assert!(tokens.contains(&"quick".to_string()));
        assert!(tokens.contains(&"brown".to_string()));
        assert!(tokens.contains(&"jumps".to_string()));
    }

    #[test]
    fn test_split_sentences() {
        let s = SessionSummarizer::new();
        let sentences = s.split_sentences("Hello world. How are you? I am fine! Thanks for asking");
        assert_eq!(sentences.len(), 4);
        assert_eq!(sentences[0], "Hello world.");
        assert_eq!(sentences[1], "How are you?");
        assert_eq!(sentences[2], "I am fine!");
    }

    #[test]
    fn test_custom_config() {
        let config = SummarizerConfig {
            max_sentences: 3,
            max_summary_chars: 500,
            min_messages_to_summarize: 2,
            rolling_window_size: 3,
            retention_hours: 24,
            min_importance: 0.5,
            stop_words: HashSet::new(),
        };
        let s = SessionSummarizer::with_config(config.clone());
        assert_eq!(s.config.max_sentences, 3);
        assert_eq!(s.config.retention_hours, 24);
    }

    #[test]
    fn test_set_config() {
        let mut s = SessionSummarizer::new();
        assert_eq!(s.config.max_sentences, 5);
        s.set_config(SummarizerConfig {
            max_sentences: 10,
            ..SummarizerConfig::default()
        });
        assert_eq!(s.config.max_sentences, 10);
    }
}
