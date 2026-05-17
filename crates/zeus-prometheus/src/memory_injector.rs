//! Memory context injector
//!
//! Searches zeus-mnemosyne for relevant memories and formats them
//! for injection into the LLM system prompt.

use tracing::debug;

/// Memory injector that searches Mnemosyne for relevant context
pub struct MemoryInjector {
    /// Maximum number of search results to include
    max_results: usize,
    /// Maximum total characters of memory context
    max_context_chars: usize,
}

impl MemoryInjector {
    pub fn new(max_results: usize, max_context_chars: usize) -> Self {
        Self {
            max_results,
            max_context_chars,
        }
    }

    /// Get the maximum context characters setting
    pub fn max_context_chars(&self) -> usize {
        self.max_context_chars
    }

    /// Get the maximum results setting
    pub fn max_results(&self) -> usize {
        self.max_results
    }

    /// Search memory for relevant context and format it for system prompt injection.
    ///
    /// Performs hierarchical weighted search across all memory types (same quality
    /// as agent loop) so cooking-loop agents get full memory context.
    ///
    /// Takes a Mnemosyne instance and the user's query.
    /// Returns a formatted string suitable for appending to the system prompt, or None if no relevant memories found.
    pub async fn fetch_context(
        &self,
        mnemosyne: &zeus_mnemosyne::Mnemosyne,
        query: &str,
    ) -> Option<String> {
        let mut weighted_entries: Vec<(f32, String)> = Vec::new();

        let fmt = |tag: &str, r: &zeus_mnemosyne::SearchResult| -> String {
            match &r.citation {
                Some(cite) => format!("[{}] {} (source: {})", tag, r.content, cite),
                None => format!("[{}] {}", tag, r.content),
            }
        };

        // 1. Working memory (session context, highest weight)
        if let Ok(results) = mnemosyne.search_by_type(query, zeus_mnemosyne::MemoryType::Working, 2).await {
            for r in results { weighted_entries.push((r.score.abs() * 3.0, fmt("working", &r))); }
        }

        // 2. Semantic memory (knowledge, high weight)
        if let Ok(results) = mnemosyne.search_by_type(query, zeus_mnemosyne::MemoryType::Semantic, 2).await {
            for r in results { weighted_entries.push((r.score.abs() * 2.0, fmt("knowledge", &r))); }
        }

        // 3. Facts (discrete knowledge, high weight)
        if let Ok(results) = mnemosyne.search_by_type(query, zeus_mnemosyne::MemoryType::Fact, 2).await {
            for r in results { weighted_entries.push((r.score.abs() * 2.0, fmt("fact", &r))); }
        }

        // 4. Preferences (user settings, high weight)
        if let Ok(results) = mnemosyne.search_by_type(query, zeus_mnemosyne::MemoryType::Preference, 1).await {
            for r in results { weighted_entries.push((r.score.abs() * 2.5, fmt("preference", &r))); }
        }

        // 5. Episodic memory (past events, standard weight)
        if let Ok(results) = mnemosyne.search_by_type(query, zeus_mnemosyne::MemoryType::Episodic, 3).await {
            for r in results { weighted_entries.push((r.score.abs(), fmt("memory", &r))); }
        }

        // Fallback: if no typed results, try untyped search
        if weighted_entries.is_empty() {
            match mnemosyne.semantic_search(query, self.max_results).await {
                Ok(r) if !r.is_empty() => {
                    for result in r {
                        weighted_entries.push((result.score.abs(), fmt("recall", &result)));
                    }
                }
                _ => {
                    if let Ok(r) = mnemosyne.search(query, self.max_results).await {
                        for result in r {
                            weighted_entries.push((result.score.abs(), fmt("recall", &result)));
                        }
                    }
                }
            }
        }

        if weighted_entries.is_empty() {
            debug!("No relevant memories found for query");
            return None;
        }

        // Sort by weighted score descending, truncate
        weighted_entries.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        weighted_entries.truncate(self.max_results);

        // Build formatted string within char budget
        let mut context = String::new();
        let mut total_chars = 0;
        for (_, entry) in &weighted_entries {
            let line = format!("{}\n", entry);
            if total_chars + line.len() > self.max_context_chars {
                break;
            }
            context.push_str(&line);
            total_chars += line.len();
        }

        if context.is_empty() {
            None
        } else {
            debug!(entries = weighted_entries.len(), chars = total_chars, "Injecting hierarchical memory context");
            Some(context.trim_end().to_string())
        }
    }

    /// Proactive context retrieval: pre-fetch memories based on conversation topics
    /// and cross-session patterns, without requiring an explicit query.
    ///
    /// Uses the current conversation messages to identify likely-needed context.
    pub async fn fetch_proactive_context(
        &self,
        mnemosyne: &zeus_mnemosyne::Mnemosyne,
        messages: &[zeus_core::Message],
    ) -> Option<String> {
        if messages.is_empty() {
            return None;
        }

        let results = match mnemosyne
            .proactive_context(messages, self.max_results)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                debug!("Proactive context retrieval failed: {}", e);
                return None;
            }
        };

        if results.is_empty() {
            return None;
        }

        debug!(results = results.len(), "Proactive context retrieved");
        Self::format_results(&results, self.max_context_chars)
    }

    /// Format search results into a context string for system prompt injection.
    fn format_results(
        results: &[zeus_mnemosyne::SearchResult],
        max_chars: usize,
    ) -> Option<String> {
        let mut context = String::new();
        let mut total_chars = 0;

        for result in results {
            let entry = format!(
                "- [{}] (relevance: {:.2}): {}\n",
                result.timestamp, result.score, result.content
            );
            if total_chars + entry.len() > max_chars {
                break;
            }
            context.push_str(&entry);
            total_chars += entry.len();
        }

        if context.is_empty() {
            None
        } else {
            debug!(
                results = results.len(),
                chars = total_chars,
                "Injecting memory context"
            );
            Some(context.trim_end().to_string())
        }
    }
}

impl Default for MemoryInjector {
    fn default() -> Self {
        Self::new(10, 8000) // Bumped from 5/4000 — more memory context for agents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_injector_defaults() {
        let injector = MemoryInjector::default();
        assert_eq!(injector.max_results, 10);
        assert_eq!(injector.max_context_chars, 8000);
    }

    #[test]
    fn test_memory_injector_custom() {
        let injector = MemoryInjector::new(10, 8000);
        assert_eq!(injector.max_results, 10);
        assert_eq!(injector.max_context_chars, 8000);
    }

    #[test]
    fn test_memory_injector_max_results_range() {
        // Verify various max_results values work
        let injector1 = MemoryInjector::new(1, 4000);
        assert_eq!(injector1.max_results, 1);

        let injector50 = MemoryInjector::new(50, 4000);
        assert_eq!(injector50.max_results, 50);

        let injector100 = MemoryInjector::new(100, 4000);
        assert_eq!(injector100.max_results, 100);
    }

    #[test]
    fn test_memory_injector_max_chars_range() {
        // Verify large max_chars values work
        let injector = MemoryInjector::new(5, 100_000);
        assert_eq!(injector.max_context_chars, 100_000);

        let injector_small = MemoryInjector::new(5, 100);
        assert_eq!(injector_small.max_context_chars, 100);
    }

    #[test]
    fn test_memory_injector_zero_results() {
        let injector = MemoryInjector::new(0, 4000);
        assert_eq!(injector.max_results, 0);
    }

    #[test]
    fn test_memory_injector_custom_builder() {
        // Test chained configuration by creating with specific values
        let injector = MemoryInjector::new(20, 16000);
        assert_eq!(injector.max_results, 20);
        assert_eq!(injector.max_context_chars, 16000);

        // Verify different from defaults
        let default = MemoryInjector::default();
        assert_ne!(injector.max_results, default.max_results);
        assert_ne!(injector.max_context_chars, default.max_context_chars);
    }

    #[test]
    fn test_memory_injector_debug() {
        // MemoryInjector doesn't derive Debug, but we can verify the fields
        // are accessible and consistent
        let injector = MemoryInjector::new(7, 3000);
        assert_eq!(injector.max_results, 7);
        assert_eq!(injector.max_context_chars, 3000);
    }

    #[test]
    fn test_memory_injector_format_results_empty() {
        // Test format_results with an empty slice
        let result = MemoryInjector::format_results(&[], 4000);
        assert!(result.is_none());
    }

    #[test]
    fn test_memory_injector_getters() {
        let injector = MemoryInjector::new(10, 8000);
        assert_eq!(injector.max_context_chars(), 8000);
        assert_eq!(injector.max_results(), 10);
    }

    #[test]
    fn test_memory_injector_default_getters() {
        let injector = MemoryInjector::default();
        assert_eq!(injector.max_context_chars(), 8000);
        assert_eq!(injector.max_results(), 10);
    }

    #[test]
    fn test_format_results_truncation() {
        use zeus_mnemosyne::{MemoryType, SearchResult};

        let results = vec![
            SearchResult {
                id: 1,
                session_id: "s1".to_string(),
                content: "Short memory".to_string(),
                timestamp: "2026-02-18T00:00:00Z".to_string(),
                score: 0.9,
                memory_type: MemoryType::Semantic,
                importance: 0.8,
                citation: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
            },
            SearchResult {
                id: 2,
                session_id: "s1".to_string(),
                content: "This is a much longer memory entry that should exceed the limit"
                    .to_string(),
                timestamp: "2026-02-18T00:00:00Z".to_string(),
                score: 0.7,
                memory_type: MemoryType::Episodic,
                importance: 0.5,
                citation: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
            },
        ];

        // With very small max_chars, only first result should fit
        let formatted = MemoryInjector::format_results(&results, 100);
        assert!(formatted.is_some());
        let text = formatted.unwrap();
        assert!(text.contains("Short memory"));
    }
}
