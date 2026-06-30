//! Session Analytics — aggregate metrics across sessions.
//!
//! Tracks and analyzes patterns across multiple sessions:
//! - **Turn metrics**: Average turns per session, tool calls per turn
//! - **Duration tracking**: Session length, response times
//! - **Tool usage**: Most/least used tools, success rates
//! - **Topic trends**: Session topic distribution over time
//! - **Engagement**: User message frequency, conversation depth
//! - **Cost tracking**: Per-session and cumulative cost aggregation

use std::collections::HashMap;

// ============================================================================
// Types
// ============================================================================

/// A session record for analytics tracking.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Session identifier.
    pub session_id: String,
    /// Number of user turns (messages).
    pub user_turns: usize,
    /// Number of assistant turns.
    pub assistant_turns: usize,
    /// Number of tool calls made.
    pub tool_calls: usize,
    /// Tool names used in this session.
    pub tools_used: Vec<String>,
    /// Number of successful tool calls.
    pub tool_successes: usize,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Session duration in seconds.
    pub duration_secs: u64,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Session start timestamp (unix secs).
    pub started_at: u64,
    /// Optional topic/category label.
    pub topic: Option<String>,
}

/// Aggregated analytics across sessions.
#[derive(Debug, Clone)]
pub struct AnalyticsSummary {
    /// Total sessions analyzed.
    pub total_sessions: usize,
    /// Average user turns per session.
    pub avg_user_turns: f64,
    /// Average assistant turns per session.
    pub avg_assistant_turns: f64,
    /// Average tool calls per session.
    pub avg_tool_calls: f64,
    /// Overall tool success rate (0.0–1.0).
    pub tool_success_rate: f64,
    /// Total cost across all sessions.
    pub total_cost_usd: f64,
    /// Average cost per session.
    pub avg_cost_per_session: f64,
    /// Average session duration in seconds.
    pub avg_duration_secs: f64,
    /// Total tokens consumed (input + output).
    pub total_tokens: u64,
    /// Most used tools (name, count).
    pub top_tools: Vec<(String, usize)>,
    /// Topic distribution (topic, count).
    pub topic_distribution: Vec<(String, usize)>,
    /// Longest session duration in seconds.
    pub max_duration_secs: u64,
    /// Shortest session duration in seconds.
    pub min_duration_secs: u64,
}

/// Time-bucketed analytics for trend analysis.
#[derive(Debug, Clone)]
pub struct TimeBucket {
    /// Bucket start timestamp (unix secs).
    pub start: u64,
    /// Bucket end timestamp (unix secs).
    pub end: u64,
    /// Number of sessions in this bucket.
    pub session_count: usize,
    /// Total cost in this bucket.
    pub cost_usd: f64,
    /// Total tool calls in this bucket.
    pub tool_calls: usize,
    /// Total tokens in this bucket.
    pub tokens: u64,
}

/// Tool usage report.
#[derive(Debug, Clone)]
pub struct ToolReport {
    /// Tool name.
    pub name: String,
    /// Total invocations.
    pub total_calls: usize,
    /// Success count.
    pub successes: usize,
    /// Success rate (0.0–1.0).
    pub success_rate: f64,
    /// Number of sessions that used this tool.
    pub sessions_used_in: usize,
}

// ============================================================================
// Analytics Engine
// ============================================================================

/// The session analytics engine.
pub struct SessionAnalytics {
    /// All tracked session records.
    records: Vec<SessionRecord>,
    /// Maximum records to retain.
    max_records: usize,
}

impl SessionAnalytics {
    /// Create a new analytics engine.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            max_records: 100_000,
        }
    }

    /// Create with a custom record limit.
    pub fn with_max_records(max_records: usize) -> Self {
        Self {
            records: Vec::new(),
            max_records,
        }
    }

    /// Add a session record.
    pub fn add_record(&mut self, record: SessionRecord) {
        self.records.push(record);
        if self.records.len() > self.max_records {
            self.records.remove(0);
        }
    }

    /// Get all records.
    pub fn records(&self) -> &[SessionRecord] {
        &self.records
    }

    /// Get number of tracked sessions.
    pub fn count(&self) -> usize {
        self.records.len()
    }

    /// Compute aggregate summary across all sessions.
    pub fn summary(&self) -> AnalyticsSummary {
        self.summary_filtered(&|_| true)
    }

    /// Compute summary for sessions matching a filter.
    pub fn summary_filtered(&self, filter: &dyn Fn(&SessionRecord) -> bool) -> AnalyticsSummary {
        let filtered: Vec<&SessionRecord> = self.records.iter().filter(|r| filter(r)).collect();

        if filtered.is_empty() {
            return AnalyticsSummary {
                total_sessions: 0,
                avg_user_turns: 0.0,
                avg_assistant_turns: 0.0,
                avg_tool_calls: 0.0,
                tool_success_rate: 0.0,
                total_cost_usd: 0.0,
                avg_cost_per_session: 0.0,
                avg_duration_secs: 0.0,
                total_tokens: 0,
                top_tools: Vec::new(),
                topic_distribution: Vec::new(),
                max_duration_secs: 0,
                min_duration_secs: 0,
            };
        }

        let n = filtered.len() as f64;
        let total_user_turns: usize = filtered.iter().map(|r| r.user_turns).sum();
        let total_assistant_turns: usize = filtered.iter().map(|r| r.assistant_turns).sum();
        let total_tool_calls: usize = filtered.iter().map(|r| r.tool_calls).sum();
        let total_tool_successes: usize = filtered.iter().map(|r| r.tool_successes).sum();
        let total_cost: f64 = filtered.iter().map(|r| r.cost_usd).sum();
        let total_duration: u64 = filtered.iter().map(|r| r.duration_secs).sum();
        let total_tokens: u64 = filtered
            .iter()
            .map(|r| r.input_tokens + r.output_tokens)
            .sum();

        let tool_success_rate = if total_tool_calls > 0 {
            total_tool_successes as f64 / total_tool_calls as f64
        } else {
            0.0
        };

        // Tool frequency
        let mut tool_freq: HashMap<String, usize> = HashMap::new();
        for r in &filtered {
            for tool in &r.tools_used {
                *tool_freq.entry(tool.clone()).or_insert(0) += 1;
            }
        }
        let mut top_tools: Vec<(String, usize)> = tool_freq.into_iter().collect();
        top_tools.sort_by(|a, b| b.1.cmp(&a.1));
        top_tools.truncate(10);

        // Topic distribution
        let mut topic_freq: HashMap<String, usize> = HashMap::new();
        for r in &filtered {
            if let Some(ref topic) = r.topic {
                *topic_freq.entry(topic.clone()).or_insert(0) += 1;
            }
        }
        let mut topic_distribution: Vec<(String, usize)> = topic_freq.into_iter().collect();
        topic_distribution.sort_by(|a, b| b.1.cmp(&a.1));

        let max_duration = filtered.iter().map(|r| r.duration_secs).max().unwrap_or(0);
        let min_duration = filtered.iter().map(|r| r.duration_secs).min().unwrap_or(0);

        AnalyticsSummary {
            total_sessions: filtered.len(),
            avg_user_turns: total_user_turns as f64 / n,
            avg_assistant_turns: total_assistant_turns as f64 / n,
            avg_tool_calls: total_tool_calls as f64 / n,
            tool_success_rate,
            total_cost_usd: total_cost,
            avg_cost_per_session: total_cost / n,
            avg_duration_secs: total_duration as f64 / n,
            total_tokens,
            top_tools,
            topic_distribution,
            max_duration_secs: max_duration,
            min_duration_secs: min_duration,
        }
    }

    /// Get time-bucketed analytics (e.g., daily/hourly).
    pub fn time_buckets(&self, bucket_size_secs: u64) -> Vec<TimeBucket> {
        if self.records.is_empty() || bucket_size_secs == 0 {
            return Vec::new();
        }

        let min_ts = self.records.iter().map(|r| r.started_at).min().unwrap_or(0);
        let max_ts = self.records.iter().map(|r| r.started_at).max().unwrap_or(0);

        let mut buckets: Vec<TimeBucket> = Vec::new();
        let mut start = min_ts - (min_ts % bucket_size_secs);

        while start <= max_ts {
            let end = start + bucket_size_secs;
            let in_bucket: Vec<&SessionRecord> = self
                .records
                .iter()
                .filter(|r| r.started_at >= start && r.started_at < end)
                .collect();

            if !in_bucket.is_empty() {
                buckets.push(TimeBucket {
                    start,
                    end,
                    session_count: in_bucket.len(),
                    cost_usd: in_bucket.iter().map(|r| r.cost_usd).sum(),
                    tool_calls: in_bucket.iter().map(|r| r.tool_calls).sum(),
                    tokens: in_bucket
                        .iter()
                        .map(|r| r.input_tokens + r.output_tokens)
                        .sum(),
                });
            }

            start = end;
        }

        buckets
    }

    /// Generate per-tool usage reports.
    pub fn tool_reports(&self) -> Vec<ToolReport> {
        let mut tool_calls: HashMap<String, usize> = HashMap::new();
        let mut tool_successes: HashMap<String, usize> = HashMap::new();
        let mut tool_sessions: HashMap<String, HashSet<usize>> = HashMap::new();

        for (idx, record) in self.records.iter().enumerate() {
            let unique_tools: HashSet<&String> = record.tools_used.iter().collect();
            for tool in &unique_tools {
                tool_sessions
                    .entry((*tool).clone())
                    .or_default()
                    .insert(idx);
            }
            for tool in &record.tools_used {
                *tool_calls.entry(tool.clone()).or_insert(0) += 1;
            }
            // Distribute successes proportionally across tools
            if !record.tools_used.is_empty() && record.tool_successes > 0 {
                let rate = record.tool_successes as f64 / record.tool_calls.max(1) as f64;
                for tool in &record.tools_used {
                    let count = tool_calls.get(tool).copied().unwrap_or(1);
                    let est_successes = (count as f64 * rate).round() as usize;
                    tool_successes.insert(tool.clone(), est_successes);
                }
            }
        }

        let mut reports: Vec<ToolReport> = tool_calls
            .into_iter()
            .map(|(name, total)| {
                let successes = tool_successes.get(&name).copied().unwrap_or(0);
                let sessions = tool_sessions.get(&name).map(|s| s.len()).unwrap_or(0);
                ToolReport {
                    name: name.clone(),
                    total_calls: total,
                    successes,
                    success_rate: if total > 0 {
                        successes as f64 / total as f64
                    } else {
                        0.0
                    },
                    sessions_used_in: sessions,
                }
            })
            .collect();

        reports.sort_by(|a, b| b.total_calls.cmp(&a.total_calls));
        reports
    }

    /// Find the most expensive sessions.
    pub fn most_expensive(&self, limit: usize) -> Vec<&SessionRecord> {
        let mut sorted: Vec<&SessionRecord> = self.records.iter().collect();
        sorted.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(limit);
        sorted
    }

    /// Find the longest sessions.
    pub fn longest_sessions(&self, limit: usize) -> Vec<&SessionRecord> {
        let mut sorted: Vec<&SessionRecord> = self.records.iter().collect();
        sorted.sort_by(|a, b| b.duration_secs.cmp(&a.duration_secs));
        sorted.truncate(limit);
        sorted
    }

    /// Get sessions by topic.
    pub fn sessions_by_topic(&self, topic: &str) -> Vec<&SessionRecord> {
        self.records
            .iter()
            .filter(|r| r.topic.as_deref() == Some(topic))
            .collect()
    }

    /// Clear all records.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl Default for SessionAnalytics {
    fn default() -> Self {
        Self::new()
    }
}

// Helper for tests
use std::collections::HashSet;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<SessionRecord> {
        vec![
            SessionRecord {
                session_id: "s1".into(),
                user_turns: 5,
                assistant_turns: 5,
                tool_calls: 3,
                tools_used: vec!["read_file".into(), "shell".into(), "read_file".into()],
                tool_successes: 3,
                cost_usd: 0.05,
                duration_secs: 120,
                input_tokens: 1000,
                output_tokens: 500,
                started_at: 1000,
                topic: Some("coding".into()),
            },
            SessionRecord {
                session_id: "s2".into(),
                user_turns: 10,
                assistant_turns: 10,
                tool_calls: 8,
                tools_used: vec![
                    "shell".into(),
                    "write_file".into(),
                    "shell".into(),
                    "web_fetch".into(),
                ],
                tool_successes: 6,
                cost_usd: 0.15,
                duration_secs: 300,
                input_tokens: 3000,
                output_tokens: 1500,
                started_at: 2000,
                topic: Some("debugging".into()),
            },
            SessionRecord {
                session_id: "s3".into(),
                user_turns: 2,
                assistant_turns: 2,
                tool_calls: 0,
                tools_used: vec![],
                tool_successes: 0,
                cost_usd: 0.01,
                duration_secs: 30,
                input_tokens: 200,
                output_tokens: 100,
                started_at: 3000,
                topic: Some("coding".into()),
            },
        ]
    }

    fn setup_analytics() -> SessionAnalytics {
        let mut a = SessionAnalytics::new();
        for r in sample_records() {
            a.add_record(r);
        }
        a
    }

    #[test]
    fn test_new_analytics() {
        let a = SessionAnalytics::new();
        assert_eq!(a.count(), 0);
        assert!(a.records().is_empty());
    }

    #[test]
    fn test_add_record() {
        let mut a = SessionAnalytics::new();
        a.add_record(sample_records().remove(0));
        assert_eq!(a.count(), 1);
    }

    #[test]
    fn test_max_records_limit() {
        let mut a = SessionAnalytics::with_max_records(2);
        for r in sample_records() {
            a.add_record(r);
        }
        assert_eq!(a.count(), 2);
        // First record should have been evicted
        assert_eq!(a.records()[0].session_id, "s2");
    }

    #[test]
    fn test_summary_basic() {
        let a = setup_analytics();
        let s = a.summary();
        assert_eq!(s.total_sessions, 3);
        assert!((s.avg_user_turns - 17.0 / 3.0).abs() < 0.01);
        assert!((s.avg_assistant_turns - 17.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_summary_tool_calls() {
        let a = setup_analytics();
        let s = a.summary();
        assert!((s.avg_tool_calls - 11.0 / 3.0).abs() < 0.01);
        // 9 successes out of 11 total
        assert!((s.tool_success_rate - 9.0 / 11.0).abs() < 0.01);
    }

    #[test]
    fn test_summary_cost() {
        let a = setup_analytics();
        let s = a.summary();
        assert!((s.total_cost_usd - 0.21).abs() < 0.001);
        assert!((s.avg_cost_per_session - 0.07).abs() < 0.001);
    }

    #[test]
    fn test_summary_duration() {
        let a = setup_analytics();
        let s = a.summary();
        assert!((s.avg_duration_secs - 150.0).abs() < 0.01);
        assert_eq!(s.max_duration_secs, 300);
        assert_eq!(s.min_duration_secs, 30);
    }

    #[test]
    fn test_summary_tokens() {
        let a = setup_analytics();
        let s = a.summary();
        assert_eq!(s.total_tokens, 1000 + 500 + 3000 + 1500 + 200 + 100);
    }

    #[test]
    fn test_summary_top_tools() {
        let a = setup_analytics();
        let s = a.summary();
        assert!(!s.top_tools.is_empty());
        // shell appears 3 times (1 in s1 + 2 in s2), read_file 2 times
        let shell_count = s
            .top_tools
            .iter()
            .find(|(n, _)| n == "shell")
            .map(|(_, c)| *c);
        assert_eq!(shell_count, Some(3));
    }

    #[test]
    fn test_summary_topics() {
        let a = setup_analytics();
        let s = a.summary();
        assert!(!s.topic_distribution.is_empty());
        let coding_count = s
            .topic_distribution
            .iter()
            .find(|(t, _)| t == "coding")
            .map(|(_, c)| *c);
        assert_eq!(coding_count, Some(2));
    }

    #[test]
    fn test_summary_empty() {
        let a = SessionAnalytics::new();
        let s = a.summary();
        assert_eq!(s.total_sessions, 0);
        assert_eq!(s.total_cost_usd, 0.0);
        assert_eq!(s.total_tokens, 0);
    }

    #[test]
    fn test_summary_filtered() {
        let a = setup_analytics();
        let s = a.summary_filtered(&|r| r.topic.as_deref() == Some("coding"));
        assert_eq!(s.total_sessions, 2);
    }

    #[test]
    fn test_time_buckets() {
        let a = setup_analytics();
        // Bucket size of 1500 secs: s1 (1000) in first, s2 (2000) in second, s3 (3000) in third
        let buckets = a.time_buckets(1500);
        assert!(!buckets.is_empty());
        let total_sessions: usize = buckets.iter().map(|b| b.session_count).sum();
        assert_eq!(total_sessions, 3);
    }

    #[test]
    fn test_time_buckets_empty() {
        let a = SessionAnalytics::new();
        assert!(a.time_buckets(3600).is_empty());
    }

    #[test]
    fn test_time_buckets_zero_size() {
        let a = setup_analytics();
        assert!(a.time_buckets(0).is_empty());
    }

    #[test]
    fn test_tool_reports() {
        let a = setup_analytics();
        let reports = a.tool_reports();
        assert!(!reports.is_empty());
        // shell should be first (3 calls)
        assert_eq!(reports[0].name, "shell");
        assert_eq!(reports[0].total_calls, 3);
        assert!(reports[0].sessions_used_in >= 1);
    }

    #[test]
    fn test_most_expensive() {
        let a = setup_analytics();
        let top = a.most_expensive(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].session_id, "s2"); // $0.15
        assert_eq!(top[1].session_id, "s1"); // $0.05
    }

    #[test]
    fn test_longest_sessions() {
        let a = setup_analytics();
        let longest = a.longest_sessions(1);
        assert_eq!(longest[0].session_id, "s2"); // 300s
    }

    #[test]
    fn test_sessions_by_topic() {
        let a = setup_analytics();
        let coding = a.sessions_by_topic("coding");
        assert_eq!(coding.len(), 2);
        let debug = a.sessions_by_topic("debugging");
        assert_eq!(debug.len(), 1);
        let empty = a.sessions_by_topic("nonexistent");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_clear() {
        let mut a = setup_analytics();
        assert_eq!(a.count(), 3);
        a.clear();
        assert_eq!(a.count(), 0);
    }

    #[test]
    fn test_tool_report_success_rate() {
        let a = setup_analytics();
        let reports = a.tool_reports();
        for report in &reports {
            assert!(report.success_rate >= 0.0 && report.success_rate <= 1.0);
        }
    }

    #[test]
    fn test_single_session_summary() {
        let mut a = SessionAnalytics::new();
        a.add_record(SessionRecord {
            session_id: "solo".into(),
            user_turns: 1,
            assistant_turns: 1,
            tool_calls: 0,
            tools_used: vec![],
            tool_successes: 0,
            cost_usd: 0.001,
            duration_secs: 5,
            input_tokens: 50,
            output_tokens: 20,
            started_at: 5000,
            topic: None,
        });
        let s = a.summary();
        assert_eq!(s.total_sessions, 1);
        assert_eq!(s.avg_user_turns, 1.0);
        assert_eq!(s.total_tokens, 70);
        assert!(s.topic_distribution.is_empty());
    }
}
