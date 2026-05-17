//! Integration tests for kimi-style orphan-segmentation repair (#57-iii).
//!
//! These tests pin the segment-scoped repair semantics landed in #57-ii:
//!   - Phase 1 uses `turn_boundary::segment_satisfied_call_ids` to compute
//!     per-segment satisfied sets (Vec<HashSet>), NOT a single global HashSet.
//!   - Phase 2 looks up satisfied IDs via `turn_segment_for_index` per
//!     assistant-with-tool_calls message.
//!   - Cascade prevention also segment-scoped — synthetic injection respects
//!     turn boundaries, never bleeds across turns.
//!
//! Before #57-ii, a global HashSet would mark any tool_call_id seen ANYWHERE
//! in the message list as "satisfied" — so when kimi-style models reuse
//! shell:0 as a tool_call_id across turns, a satisfied call in turn-1 would
//! mask a real orphan in turn-2 (or vice versa).
//!
//! Catches operationalized:
//!   - #34 (cargo invariant): tests run under `cargo test -p zeus-agent`
//!   - #45 (DRY): tests exercise the single zeus_session entry point
//!   - #53 (verbatim): test assertions are explicit, no narrative
//!   - #57 family: segment-scope is the load-bearing invariant under test

use chrono::Utc;
use zeus_core::{Message, Role, TextDirection, ToolCall, ToolResult};
use zeus_session::repair_orphaned_tool_calls;

// ============================================================================
// Fixture helpers
// ============================================================================

fn user_msg(content: &str) -> Message {
    Message {
        role: Role::User,
        content: content.to_string(),
        tool_calls: vec![],
        tool_results: vec![],
        timestamp: Utc::now(),
        attachments: vec![],
        message_id: None,
        parent_id: None,
        thread_id: None,
        direction: TextDirection::Ltr,
        channel_source: None,
        compaction_hint: Default::default(),
    }
}

fn assistant_with_calls(content: &str, calls: Vec<(&str, &str)>) -> Message {
    Message {
        role: Role::Assistant,
        content: content.to_string(),
        tool_calls: calls
            .into_iter()
            .map(|(id, name)| ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: serde_json::json!({}),
            })
            .collect(),
        tool_results: vec![],
        timestamp: Utc::now(),
        attachments: vec![],
        message_id: None,
        parent_id: None,
        thread_id: None,
        direction: TextDirection::Ltr,
        channel_source: None,
        compaction_hint: Default::default(),
    }
}

fn tool_msg(results: Vec<(&str, &str)>) -> Message {
    Message {
        role: Role::Tool,
        content: String::new(),
        tool_calls: vec![],
        tool_results: results
            .into_iter()
            .map(|(call_id, output)| ToolResult {
                call_id: call_id.to_string(),
                success: true,
                output: output.to_string(),
            })
            .collect(),
        timestamp: Utc::now(),
        attachments: vec![],
        message_id: None,
        parent_id: None,
        thread_id: None,
        direction: TextDirection::Ltr,
        channel_source: None,
        compaction_hint: Default::default(),
    }
}

/// Count synthetic tool_results in a message list.
///
/// Synthetic results are injected by `repair_orphaned_tool_calls` with
/// `success: false` and a specific corruption-notice output literal.
/// We match on the structural invariant (`success: false`) rather than
/// the output string to stay robust to wording changes in the repair body.
fn count_synthetic(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|m| m.tool_results.iter())
        .filter(|tr| !tr.success)
        .count()
}

/// Find a tool_result by call_id across all messages.
fn find_result<'a>(messages: &'a [Message], call_id: &str) -> Option<&'a ToolResult> {
    messages
        .iter()
        .flat_map(|m| m.tool_results.iter())
        .find(|tr| tr.call_id == call_id)
}

// ============================================================================
// Test 1: kimi shell:0 cross-turn reuse
// ============================================================================

/// Construct two turns where both reuse `shell:0` as tool_call_id.
/// Turn-1 has a matching result (satisfied). Turn-2 has NO result (orphan).
///
/// Pre-#57-ii bug: global HashSet sees turn-1's shell:0 in the satisfied
/// set and skips synthetic injection for turn-2 — leaving a real orphan
/// in the message list. Post-#57-ii: per-segment HashSets isolate turn-1
/// and turn-2 satisfied sets, so turn-2's shell:0 is correctly identified
/// as orphan and gets a synthetic.
#[test]
fn kimi_shell_0_cross_turn_reuse() {
    let mut messages = vec![
        // ---- Turn 1: satisfied ----
        user_msg("turn 1 user request"),
        assistant_with_calls("calling shell", vec![("shell:0", "shell")]),
        tool_msg(vec![("shell:0", "turn-1 result OK")]),
        // ---- Turn 2: shell:0 reused, NO result (orphan) ----
        user_msg("turn 2 user request"),
        assistant_with_calls("calling shell again", vec![("shell:0", "shell")]),
        // No tool_msg follows — this shell:0 is orphan within turn-2's segment.
    ];

    let before_synthetic = count_synthetic(&messages);
    repair_orphaned_tool_calls(&mut messages, None);
    let after_synthetic = count_synthetic(&messages);

    // Turn-2's shell:0 should now have a synthetic result injected.
    assert!(
        after_synthetic > before_synthetic,
        "expected synthetic injection for turn-2 orphan shell:0; before={before_synthetic} after={after_synthetic}"
    );

    // Turn-1's real result must still be present and unchanged.
    let turn1_result = messages
        .iter()
        .flat_map(|m| m.tool_results.iter())
        .find(|tr| tr.call_id == "shell:0" && tr.output == "turn-1 result OK");
    assert!(
        turn1_result.is_some(),
        "turn-1's real shell:0 result must survive repair (segment-isolation invariant)"
    );
}

// ============================================================================
// Test 2: segment-boundary edges
// ============================================================================

/// Tool call at end of segment N, result at start of segment N+1.
/// Per turn_boundary semantics, an assistant tool_call followed by a tool
/// message belongs to segment N. The NEXT user message opens segment N+1.
///
/// This test pins that the orphan detector correctly attributes the
/// satisfied call_id to segment N, NOT segment N+1.
#[test]
fn segment_boundary_edges() {
    let mut messages = vec![
        user_msg("turn 1"),
        assistant_with_calls("end-of-segment call", vec![("call:edge", "shell")]),
        tool_msg(vec![("call:edge", "result at segment boundary")]),
        // Segment 2 begins here
        user_msg("turn 2"),
        assistant_with_calls("turn 2 reply, no tools", vec![]),
    ];

    let before_len = messages.len();
    repair_orphaned_tool_calls(&mut messages, None);

    // No synthetic should be injected — the call:edge has a real result.
    let synthetic = count_synthetic(&messages);
    assert_eq!(
        synthetic, 0,
        "no synthetic should be injected when call has real result, even at segment boundary"
    );

    // No phantom messages inserted.
    assert_eq!(
        messages.len(),
        before_len,
        "message count must be stable when no repair is needed"
    );

    // Real result still findable by call_id.
    let r = find_result(&messages, "call:edge");
    assert!(r.is_some(), "call:edge result must be retained");
    assert_eq!(r.unwrap().output, "result at segment boundary");
}

// ============================================================================
// Test 3: cascade prevention (segment-scoped)
// ============================================================================

/// Multiple assistant messages within a single segment, mix of satisfied
/// and orphan calls. Repair must only inject synthetics for the true
/// orphans, never cascade-fabricate results for already-satisfied calls.
///
/// Pre-#57 cascade bug: if Phase 1 checked only `i+1` for results, a
/// later real result could be mistakenly matched against a synthetic
/// already injected — producing duplicate/cascading synthetics.
/// Post-#57-ii: per-segment HashSets prevent this.
#[test]
fn cascade_prevention_segment_scoped() {
    let mut messages = vec![
        user_msg("turn with mixed orphans"),
        assistant_with_calls(
            "two calls",
            vec![("call:satisfied", "shell"), ("call:orphan", "shell")],
        ),
        // Only one result — call:satisfied is satisfied, call:orphan is orphan.
        tool_msg(vec![("call:satisfied", "real result")]),
    ];

    repair_orphaned_tool_calls(&mut messages, None);

    // Exactly ONE synthetic should be injected (for call:orphan).
    let synthetic = count_synthetic(&messages);
    assert_eq!(
        synthetic, 1,
        "exactly one synthetic for the orphan; satisfied call must not get duplicate (cascade prevention)"
    );

    // The real result for call:satisfied must still be present, unmodified.
    let real = messages
        .iter()
        .flat_map(|m| m.tool_results.iter())
        .find(|tr| tr.call_id == "call:satisfied" && tr.output == "real result");
    assert!(
        real.is_some(),
        "real result for call:satisfied must survive (no cascade contamination)"
    );

    // The synthetic must be for call:orphan, not call:satisfied.
    let orphan_synth = messages
        .iter()
        .flat_map(|m| m.tool_results.iter())
        .find(|tr| tr.call_id == "call:orphan");
    assert!(
        orphan_synth.is_some(),
        "synthetic must be injected for call:orphan specifically"
    );
}

// ============================================================================
// Test 4: repair end-to-end via single source of truth
// ============================================================================

/// Verify the locus-4 DRY collapse from #57-ii: zeus-agent and zeus-session
/// share the SAME repair function. Constructs an orphan scenario, repairs
/// via the public `zeus_session::repair_orphaned_tool_calls` (the single
/// source of truth all 4 call-sites now share post-#57-ii locus-4 inline-
/// replace), and verifies the post-state is consistent + idempotent.
#[test]
fn repair_end_to_end_idempotent() {
    let mut messages = vec![
        user_msg("orphan scenario"),
        assistant_with_calls("orphan call", vec![("call:lonely", "shell")]),
        // No tool_msg follows.
    ];

    // First repair: should inject one synthetic.
    repair_orphaned_tool_calls(&mut messages, None);
    let synthetic_after_1 = count_synthetic(&messages);
    assert_eq!(synthetic_after_1, 1, "first repair injects one synthetic");

    let len_after_1 = messages.len();

    // Second repair: should be idempotent — synthetic already satisfies
    // the orphan, so no further injections.
    repair_orphaned_tool_calls(&mut messages, None);
    let synthetic_after_2 = count_synthetic(&messages);
    let len_after_2 = messages.len();

    assert_eq!(
        synthetic_after_2, synthetic_after_1,
        "repair must be idempotent — no duplicate synthetics on second pass"
    );
    assert_eq!(
        len_after_2, len_after_1,
        "message count must be stable across repeated repairs (idempotency invariant)"
    );

    // The orphan call_id must now be satisfied.
    let satisfied = find_result(&messages, "call:lonely");
    assert!(
        satisfied.is_some(),
        "call:lonely must have a result (synthetic or otherwise) after repair"
    );
}
