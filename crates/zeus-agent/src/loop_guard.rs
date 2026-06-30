//! Loop Guard — Enhanced tool-call loop detection for the agent loop.
//!
//! Three complementary mechanisms:
//!
//! 1. **Per-hash counter** — SHA-256(tool_name + canonical_args) tracks how many
//!    times the exact same call (name + args) has been made this turn. Fires a
//!    warning at `per_hash_threshold` (default 3) and blocks at 2× that value.
//!
//! 2. **Ping-pong detector** — Inspects the last 4 call hashes.  If they form
//!    the pattern A → B → A → B (two distinct hashes alternating), the agent is
//!    bouncing between two tools without progress.
//!
//! 3. **Global circuit breaker** — Aborts unconditionally once `global_limit`
//!    (default 50) total tool calls have been made in a single agent turn.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::warn;

// ============================================================================
// Public types
// ============================================================================

/// Decision returned by [`LoopGuard::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopGuardVerdict {
    /// Call is fine — proceed normally.
    Allow,
    /// Call looks suspicious — inject the message as a system warning but
    /// still execute the tool.
    Warn(String),
    /// Call is blocked — do NOT execute the tool; return the message as the
    /// tool error output.
    Block(String),
}

impl LoopGuardVerdict {
    pub fn is_block(&self) -> bool {
        matches!(self, Self::Block(_))
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Warn(m) | Self::Block(m) => Some(m.as_str()),
            Self::Allow => None,
        }
    }
}

// ============================================================================
// LoopGuard
// ============================================================================

/// Stateful guard that tracks tool calls within one agent turn.
///
/// Create one per agent run and call [`LoopGuard::check`] before every tool
/// execution.  Call [`LoopGuard::reset`] between turns.
#[derive(Debug, Clone)]
pub struct LoopGuard {
    /// Total tool invocations this turn (all tools, all args).
    total_calls: usize,
    /// How many times each (tool, args) hash has been called.
    hash_counts: HashMap<String, usize>,
    /// Ring-buffer of the last `PINGPONG_WINDOW` call hashes (oldest first).
    recent_hashes: Vec<String>,
    /// Per-identical-call threshold before a warning fires.
    per_hash_threshold: usize,
    /// Global circuit-breaker limit for the entire turn.
    global_limit: usize,
}

/// Number of recent hashes retained for ping-pong analysis.
const PINGPONG_WINDOW: usize = 6;

impl LoopGuard {
    /// Create a guard with explicit thresholds.
    pub fn new(per_hash_threshold: usize, global_limit: usize) -> Self {
        Self {
            total_calls: 0,
            hash_counts: HashMap::new(),
            recent_hashes: Vec::with_capacity(PINGPONG_WINDOW + 1),
            per_hash_threshold: per_hash_threshold.max(2),
            global_limit: global_limit.max(1),
        }
    }

    /// Default: warn after 3 identical calls, circuit-break at 200 total.
    pub fn default_limits() -> Self {
        Self::new(3, 200)
    }

    /// Reset all state.  Call this at the start of each new agent turn.
    pub fn reset(&mut self) {
        self.total_calls = 0;
        self.hash_counts.clear();
        self.recent_hashes.clear();
    }

    /// Total tool calls recorded this turn.
    pub fn total_calls(&self) -> usize {
        self.total_calls
    }

    // ── Core check ──────────────────────────────────────────────────────────

    /// Record a prospective tool call and decide whether to allow, warn, or
    /// block it.
    ///
    /// Call this **before** executing the tool.  The state is updated
    /// regardless of the verdict so counters stay accurate.
    pub fn check(&mut self, tool_name: &str, args: &serde_json::Value) -> LoopGuardVerdict {
        self.total_calls += 1;

        // ── 1. Global circuit breaker ────────────────────────────────────
        if self.total_calls > self.global_limit {
            let msg = format!(
                "CIRCUIT BREAKER: {} total tool calls have been made this turn (limit: {}). \
                 The agent turn is being aborted to prevent runaway execution. \
                 Please break your task into smaller steps.",
                self.total_calls, self.global_limit
            );
            warn!("{}", msg);
            return LoopGuardVerdict::Block(msg);
        }

        // ── 2. Per-hash counter ──────────────────────────────────────────
        let hash = Self::call_hash(tool_name, args);
        let count = self.hash_counts.entry(hash.clone()).or_insert(0);
        *count += 1;
        let current_count = *count;

        // Update recent-hash window (capped at PINGPONG_WINDOW)
        self.recent_hashes.push(hash.clone());
        if self.recent_hashes.len() > PINGPONG_WINDOW {
            self.recent_hashes.remove(0);
        }

        // Hard block at 2× threshold
        let hard_block = self.per_hash_threshold * 2;
        if current_count >= hard_block {
            let msg = format!(
                "LOOP BLOCKED: tool '{}' with identical arguments has been called {} times \
                 this turn (hard limit: {}). Execution halted. \
                 Try a different approach or different arguments.",
                tool_name, current_count, hard_block
            );
            warn!("{}", msg);
            return LoopGuardVerdict::Block(msg);
        }

        // Soft warning at threshold
        if current_count == self.per_hash_threshold {
            let msg = format!(
                "WARNING: tool '{}' with identical arguments has been called {} times \
                 this turn. This may indicate a loop — consider changing your approach \
                 or using different arguments.",
                tool_name, current_count
            );
            warn!("{}", msg);
            return LoopGuardVerdict::Warn(msg);
        }

        // ── 3. Ping-pong detection ───────────────────────────────────────
        if let Some(msg) = self.detect_pingpong(tool_name) {
            return LoopGuardVerdict::Warn(msg);
        }

        LoopGuardVerdict::Allow
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    /// Compute SHA-256(tool_name + canonical JSON args) → hex string.
    fn call_hash(tool_name: &str, args: &serde_json::Value) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"\x00"); // separator
        // Use compact, sorted-key JSON for a stable canonical form
        let canonical = Self::canonical_json(args);
        hasher.update(canonical.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Produce a canonical (sorted-key) JSON string for stable hashing.
    fn canonical_json(value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::Object(map) => {
                let mut pairs: Vec<(&String, &serde_json::Value)> = map.iter().collect();
                pairs.sort_by_key(|(k, _)| k.as_str());
                let inner: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("\"{}\":{}", k, Self::canonical_json(v)))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            serde_json::Value::Array(arr) => {
                let inner: Vec<String> = arr.iter().map(Self::canonical_json).collect();
                format!("[{}]", inner.join(","))
            }
            other => other.to_string(),
        }
    }

    /// Detect A→B→A→B ping-pong in `recent_hashes`.
    ///
    /// Pattern: last 4 hashes satisfy `h[n-4] == h[n-2]` and
    /// `h[n-3] == h[n-1]` and `h[n-4] != h[n-3]`.
    fn detect_pingpong(&self, tool_name: &str) -> Option<String> {
        let h = &self.recent_hashes;
        if h.len() < 4 {
            return None;
        }
        let n = h.len();
        let a = &h[n - 4];
        let b = &h[n - 3];
        let c = &h[n - 2];
        let d = &h[n - 1];

        if a == c && b == d && a != b {
            let msg = format!(
                "PING-PONG LOOP detected: tool '{}' is part of an A→B→A→B alternating \
                 call pattern. The last 4 tool calls have been oscillating between two \
                 identical (tool, args) combinations without progress. \
                 Please try a completely different approach.",
                tool_name
            );
            warn!("{}", msg);
            return Some(msg);
        }
        None
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn guard() -> LoopGuard {
        LoopGuard::new(3, 50)
    }

    // ── Per-hash threshold ───────────────────────────────────────────────────

    #[test]
    fn test_allow_distinct_calls() {
        let mut g = guard();
        assert_eq!(
            g.check("shell", &json!({"command": "ls"})),
            LoopGuardVerdict::Allow
        );
        assert_eq!(
            g.check("read_file", &json!({"path": "/tmp/foo"})),
            LoopGuardVerdict::Allow
        );
        assert_eq!(
            g.check("shell", &json!({"command": "pwd"})),
            LoopGuardVerdict::Allow
        );
    }

    #[test]
    fn test_warn_at_threshold() {
        let mut g = guard(); // threshold = 3
        let args = json!({"command": "ls -la"});
        g.check("shell", &args);
        g.check("shell", &args);
        let v = g.check("shell", &args);
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));
        assert!(v.message().unwrap().contains("shell"));
    }

    #[test]
    fn test_block_at_double_threshold() {
        let mut g = guard(); // hard block at 6
        let args = json!({"command": "ls"});
        for _ in 0..5 {
            g.check("shell", &args);
        }
        let v = g.check("shell", &args);
        assert!(v.is_block());
        assert!(v.message().unwrap().contains("LOOP BLOCKED"));
    }

    #[test]
    fn test_different_args_are_independent() {
        let mut g = guard();
        g.check("shell", &json!({"command": "ls"}));
        g.check("shell", &json!({"command": "ls"}));
        // Different args — should be Allow
        let v = g.check("shell", &json!({"command": "pwd"}));
        assert_eq!(v, LoopGuardVerdict::Allow);
    }

    // ── Global circuit breaker ───────────────────────────────────────────────

    #[test]
    fn test_circuit_breaker_fires_at_limit() {
        let mut g = LoopGuard::new(3, 5); // global limit = 5
        for i in 0..5 {
            let v = g.check("shell", &json!({"command": format!("cmd{}", i)}));
            assert!(!v.is_block(), "should not block before limit");
        }
        // 6th call → circuit breaker
        let v = g.check("shell", &json!({"command": "extra"}));
        assert!(v.is_block());
        assert!(v.message().unwrap().contains("CIRCUIT BREAKER"));
    }

    #[test]
    fn test_circuit_breaker_after_reset() {
        let mut g = LoopGuard::new(3, 3);
        for i in 0..3 {
            g.check("shell", &json!({"command": format!("cmd{}", i)}));
        }
        g.reset();
        // After reset, counter starts over
        let v = g.check("shell", &json!({"command": "cmd0"}));
        assert_eq!(v, LoopGuardVerdict::Allow);
        assert_eq!(g.total_calls(), 1);
    }

    // ── Ping-pong detection ──────────────────────────────────────────────────

    #[test]
    fn test_pingpong_detected() {
        let mut g = guard();
        let a = json!({"command": "ls"});
        let b = json!({"command": "pwd"});
        g.check("shell", &a);
        g.check("shell", &b);
        g.check("shell", &a);
        let v = g.check("shell", &b);
        // 4th call completes A→B→A→B
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));
        assert!(v.message().unwrap().contains("PING-PONG"));
    }

    #[test]
    fn test_no_pingpong_without_pattern() {
        let mut g = guard();
        g.check("shell", &json!({"command": "ls"}));
        g.check("read_file", &json!({"path": "/a"}));
        g.check("write_file", &json!({"path": "/b"}));
        let v = g.check("shell", &json!({"command": "ls"}));
        // Not A→B→A→B
        assert_eq!(v, LoopGuardVerdict::Allow);
    }

    // ── Canonical JSON ───────────────────────────────────────────────────────

    #[test]
    fn test_canonical_json_key_order_independent() {
        let a = json!({"z": 1, "a": 2});
        let b = json!({"a": 2, "z": 1});
        let ha = LoopGuard::call_hash("tool", &a);
        let hb = LoopGuard::call_hash("tool", &b);
        assert_eq!(ha, hb, "different key order must produce same hash");
    }

    #[test]
    fn test_different_tools_different_hash() {
        let args = json!({"command": "ls"});
        let h1 = LoopGuard::call_hash("shell", &args);
        let h2 = LoopGuard::call_hash("read_file", &args);
        assert_ne!(h1, h2);
    }

    // ── Verdict helpers ──────────────────────────────────────────────────────

    #[test]
    fn test_verdict_is_block() {
        assert!(LoopGuardVerdict::Block("x".into()).is_block());
        assert!(!LoopGuardVerdict::Warn("x".into()).is_block());
        assert!(!LoopGuardVerdict::Allow.is_block());
    }

    #[test]
    fn test_verdict_message() {
        assert_eq!(LoopGuardVerdict::Allow.message(), None);
        assert_eq!(LoopGuardVerdict::Warn("hi".into()).message(), Some("hi"));
        assert_eq!(LoopGuardVerdict::Block("bye".into()).message(), Some("bye"));
    }
}
