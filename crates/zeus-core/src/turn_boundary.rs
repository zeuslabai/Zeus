//! Turn-boundary segmentation helpers.
//!
//! These helpers exist to fix a class of bugs where tool_call/tool_result
//! pairing is scoped globally across a conversation instead of per-turn.
//! When upstream providers (e.g. Kimi/Moonshot k2.6) reuse `tool_call.id`
//! values across turns (e.g. `shell:0` in turn 1 AND turn 3), global pairing
//! masks orphan tool_calls in later turns by matching them against earlier
//! turns' tool_results.
//!
//! A "turn" is bounded by [`Role::User`] messages: each user message starts
//! a new turn-segment. The first segment may be empty or contain only
//! system/assistant priming.
//!
//! See #57 for the kimi-orphan-segmentation incident that motivated this
//! module.
//!
//! # Invariants
//!
//! - `segment_satisfied_call_ids(msgs).len() == segment_pending_call_ids(msgs).len()`
//!   (both return one `HashSet` per turn-segment in the same order)
//! - For any valid `idx < msgs.len()`, `turn_segment_for_index(msgs, idx)` returns
//!   an index into the segment vectors above.
//! - Segmentation is stable: adding messages to a later turn does not change
//!   earlier segments' contents.

use std::collections::HashSet;

use crate::{Message, Role};

/// Returns the index of the turn-segment containing message at `idx`.
///
/// Segment 0 covers any messages before the first [`Role::User`] message
/// (typically system + priming). Each subsequent [`Role::User`] starts a
/// new segment that extends until the next [`Role::User`] or end-of-stream.
///
/// # Panics
///
/// Does not panic. If `idx >= messages.len()`, returns the last segment
/// index (or 0 if `messages` is empty).
pub fn turn_segment_for_index(messages: &[Message], idx: usize) -> usize {
    let mut segment = 0usize;
    for (i, m) in messages.iter().enumerate() {
        // Boundary check: each User role (except at i==0) starts a new segment.
        if i > 0 && m.role == Role::User {
            segment += 1;
        }
        if i == idx {
            return segment;
        }
    }
    // idx >= len: return final segment index
    segment
}

/// Returns one `HashSet<String>` per turn-segment, each containing the
/// `tool_result.call_id` values that appear within that segment.
///
/// "Satisfied" call_ids are those for which a tool_result exists in the
/// same segment — these are the IDs that an orphan-check should consider
/// "paired" within that segment.
pub fn segment_satisfied_call_ids(messages: &[Message]) -> Vec<HashSet<String>> {
    let mut segments: Vec<HashSet<String>> = vec![HashSet::new()];
    for (i, m) in messages.iter().enumerate() {
        if i > 0 && m.role == Role::User {
            segments.push(HashSet::new());
        }
        let last = segments
            .last_mut()
            .expect("segments always has at least one entry");
        for tr in &m.tool_results {
            last.insert(tr.call_id.clone());
        }
    }
    segments
}

/// Returns one `HashSet<String>` per turn-segment, each containing the
/// `tool_call.id` values that appear within that segment.
///
/// "Pending" call_ids are tool_call IDs emitted within a segment; the
/// inverse direction of [`segment_satisfied_call_ids`]. Used by atomic
/// compaction pair-drop logic to scope pair-retention per-segment.
pub fn segment_pending_call_ids(messages: &[Message]) -> Vec<HashSet<String>> {
    let mut segments: Vec<HashSet<String>> = vec![HashSet::new()];
    for (i, m) in messages.iter().enumerate() {
        if i > 0 && m.role == Role::User {
            segments.push(HashSet::new());
        }
        let last = segments
            .last_mut()
            .expect("segments always has at least one entry");
        for tc in &m.tool_calls {
            last.insert(tc.id.clone());
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolCall, ToolResult};
    use chrono::Utc;

    fn msg(role: Role, tcs: Vec<&str>, trs: Vec<&str>) -> Message {
        Message {
            role,
            content: String::new(),
            tool_calls: tcs
                .into_iter()
                .map(|id| ToolCall {
                    id: id.to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::Value::Null,
                })
                .collect(),
            tool_results: trs
                .into_iter()
                .map(|id| ToolResult {
                    call_id: id.to_string(),
                    success: true,
                    output: String::new(),
                })
                .collect(),
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: Default::default(),
            channel_source: None,
            compaction_hint: Default::default(),
        }
    }

    #[test]
    fn segments_satisfied_across_user_boundaries() {
        // System | User1 -> Assistant(tc:a) -> Tool(tr:a) | User2 -> Assistant(tc:b) -> Tool(tr:b)
        let msgs = vec![
            msg(Role::System, vec![], vec![]),
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["a"], vec![]),
            msg(Role::Tool, vec![], vec!["a"]),
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["b"], vec![]),
            msg(Role::Tool, vec![], vec!["b"]),
        ];
        let segs = segment_satisfied_call_ids(&msgs);
        assert_eq!(segs.len(), 3, "system-prefix + 2 user-bounded segments");
        assert!(segs[0].is_empty(), "system prefix has no tool_results");
        assert_eq!(segs[1], HashSet::from(["a".to_string()]));
        assert_eq!(segs[2], HashSet::from(["b".to_string()]));
    }

    #[test]
    fn segments_pending_across_user_boundaries() {
        let msgs = vec![
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["shell:0", "shell:1"], vec![]),
            msg(Role::Tool, vec![], vec!["shell:0"]),
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["shell:0"], vec![]),
        ];
        let segs = segment_pending_call_ids(&msgs);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], HashSet::from(["shell:0".to_string(), "shell:1".to_string()]));
        assert_eq!(segs[1], HashSet::from(["shell:0".to_string()]));
    }

    #[test]
    fn reused_call_id_across_turns_is_segment_scoped() {
        // Kimi-style reuse: shell:0 appears in both turn 1 and turn 2.
        // Turn 1: tool_call shell:0 satisfied by tool_result shell:0.
        // Turn 2: tool_call shell:0 has NO matching tool_result in same segment.
        // Global scoping would falsely mark turn-2 shell:0 as satisfied.
        let msgs = vec![
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["shell:0"], vec![]),
            msg(Role::Tool, vec![], vec!["shell:0"]),
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["shell:0"], vec![]),
        ];
        let satisfied = segment_satisfied_call_ids(&msgs);
        assert_eq!(satisfied.len(), 2);
        assert_eq!(satisfied[0], HashSet::from(["shell:0".to_string()]));
        assert!(
            satisfied[1].is_empty(),
            "turn 2 shell:0 must NOT be satisfied by turn 1's tool_result"
        );
    }

    #[test]
    fn turn_segment_for_index_locates_correctly() {
        let msgs = vec![
            msg(Role::System, vec![], vec![]),     // 0 → seg 0
            msg(Role::User, vec![], vec![]),       // 1 → seg 1
            msg(Role::Assistant, vec!["a"], vec![]), // 2 → seg 1
            msg(Role::Tool, vec![], vec!["a"]),    // 3 → seg 1
            msg(Role::User, vec![], vec![]),       // 4 → seg 2
            msg(Role::Assistant, vec!["b"], vec![]), // 5 → seg 2
        ];
        assert_eq!(turn_segment_for_index(&msgs, 0), 0);
        assert_eq!(turn_segment_for_index(&msgs, 1), 1);
        assert_eq!(turn_segment_for_index(&msgs, 2), 1);
        assert_eq!(turn_segment_for_index(&msgs, 3), 1);
        assert_eq!(turn_segment_for_index(&msgs, 4), 2);
        assert_eq!(turn_segment_for_index(&msgs, 5), 2);
        // Out-of-bounds returns last segment.
        assert_eq!(turn_segment_for_index(&msgs, 999), 2);
    }

    #[test]
    fn empty_messages_returns_single_empty_segment() {
        let segs = segment_satisfied_call_ids(&[]);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].is_empty());

        let pending = segment_pending_call_ids(&[]);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].is_empty());

        assert_eq!(turn_segment_for_index(&[], 0), 0);
    }

    #[test]
    fn first_message_user_starts_segment_one() {
        // If the first message is User (no system prefix), segment 0 is empty
        // and segment 1 starts at index 0... but actually segment 0 contains
        // index 0 (we don't increment for the first message regardless of role).
        let msgs = vec![
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["a"], vec![]),
            msg(Role::Tool, vec![], vec!["a"]),
        ];
        assert_eq!(turn_segment_for_index(&msgs, 0), 0);
        assert_eq!(turn_segment_for_index(&msgs, 1), 0);
        assert_eq!(turn_segment_for_index(&msgs, 2), 0);
        let satisfied = segment_satisfied_call_ids(&msgs);
        assert_eq!(satisfied.len(), 1);
        assert_eq!(satisfied[0], HashSet::from(["a".to_string()]));
    }

    #[test]
    fn multiple_tool_calls_same_segment_all_tracked() {
        let msgs = vec![
            msg(Role::User, vec![], vec![]),
            msg(Role::Assistant, vec!["a", "b", "c"], vec![]),
            msg(Role::Tool, vec![], vec!["a", "b"]),
        ];
        let satisfied = segment_satisfied_call_ids(&msgs);
        let pending = segment_pending_call_ids(&msgs);
        assert_eq!(satisfied[0], HashSet::from(["a".to_string(), "b".to_string()]));
        assert_eq!(
            pending[0],
            HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()])
        );
        // "c" is in pending but not satisfied — orphan in this segment.
    }
}
