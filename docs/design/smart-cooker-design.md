# Smart Cooker Design Doc

**Author:** Zeus112  
**Date:** 2026-04-23  
**Status:** Draft  

## Problem

The Zeus cooking loop executes tasks but lacks intelligence about:
1. Whether a task actually completed (code pushed? tests pass?)
2. When to stop cooking a stale/irrelevant task
3. How to advance the task queue after completion
4. How to adapt iteration budgets based on task complexity

The heartbeat cooks tasks blindly — it doesn't know if the work is done, if the task is stale, or if it should move on.

## Current State

### Cooking Loop (`tool_executor.rs:484-992`)
- **Hard limits:** 20 iterations, 50 tool calls
- **Planning-only retry:** Detects "I will..." without action, injects "act now" (max 2 retries)
- **Per-tool timeout:** 120s prevents hung tools
- **LoopGuard:** Blocks identical repeated calls
- **Auto-compaction:** 3-stage context management (summarize → hard rotate → degrade)
- **Error recovery:** Exponential backoff + profile rotation for auth/billing errors

### Heartbeat Tasks (`heartbeat.rs:856-1077`)
- **Iteration budget:** 5 (too low for dev tasks)
- **Completion signal:** Only `HEARTBEAT_OK` exact match
- **No auto-advance:** CURRENT TASK stays forever even after completion
- **No stale detection:** Tasks from 3 days ago still cook every tick

## Design

### 1. Adaptive Iteration Budget

Instead of fixed 5 iterations for all heartbeat tasks, adapt based on task type:

```rust
fn compute_iteration_budget(task: &str, current_task: &str) -> usize {
    let is_dev = current_task.contains(".rs")
        || current_task.contains("crates/")
        || current_task.contains("cargo")
        || current_task.contains("commit")
        || current_task.contains("push");
    
    if is_dev { 15 }          // Dev tasks need room for code → test → commit → push
    else if task.contains("research") { 10 }  // Research tasks need web_fetch + analysis
    else { 5 }                // Routine tasks (status check, push work) stay lean
}
```

**~5 LOC in `execute_heartbeat_task()`.**

### 2. Task Completion Detection

After a heartbeat cook completes, use LLM to evaluate whether the CURRENT TASK is done:

```rust
async fn detect_task_completion(
    llm: &LlmClient,
    task_description: &str,
    cook_result: &str,
) -> TaskStatus {
    let prompt = format!(
        "You just ran a heartbeat task. Evaluate the result:\n\n\
         TASK: {}\n\
         RESULT: {}\n\n\
         Is this task COMPLETE, IN_PROGRESS, or BLOCKED?\n\
         Reply with exactly one of: COMPLETE, IN_PROGRESS, BLOCKED\n\
         If COMPLETE, also say what was delivered (e.g. commit hash, file created).",
        task_description, cook_result
    );
    
    let response = llm.complete(&[Message::user(&prompt)], &[], None).await;
    // Parse response into TaskStatus enum
}
```

**~20 LOC. Lightweight LLM call (no tools, ~200 tokens).**

### 3. Auto-Advance Task Queue

After completion detection returns `COMPLETE`:

```rust
if status == TaskStatus::Complete {
    // Move completed task to ## COMPLETED with timestamp
    workspace.advance_task_queue().await?;
    info!("Task completed, advanced queue");
    
    // Deliver completion notice
    if let Some(ref tx) = result_tx {
        tx.try_send(format!("[Task Complete] {}: {}", task_name, deliverable));
    }
}
```

**~10 LOC in `heartbeat_loop()` after cook result processing.**

### 4. Stale Task Pruning

Tasks older than a configurable TTL with no progress should be flagged:

```rust
struct HeartbeatState {
    last_run: HashMap<String, u64>,
    last_output: HashMap<String, String>,
    // NEW: track when CURRENT TASK was set
    current_task_set_at: Option<u64>,
    current_task_last_progress: Option<u64>,
}

fn is_task_stale(state: &HeartbeatState, now: u64, ttl_secs: u64) -> bool {
    if let Some(set_at) = state.current_task_set_at {
        let age = now.saturating_sub(set_at);
        let since_progress = state.current_task_last_progress
            .map(|p| now.saturating_sub(p))
            .unwrap_or(age);
        
        // Task is stale if: older than TTL AND no progress in last TTL/2
        age > ttl_secs && since_progress > ttl_secs / 2
    } else {
        false
    }
}
```

**~15 LOC. Default TTL: 6 hours. Stale tasks get moved to COMPLETED with "(stale — auto-cleared)" note.**

### 5. LLM Pre-Check (Smart Skip)

Before cooking a task, do a lightweight check:

```rust
async fn should_cook_task(
    llm: &LlmClient,
    workspace: &Workspace,
    task: &str,
) -> bool {
    // Quick checks first (no LLM needed)
    if task.contains("push") {
        // Check git status — if clean, skip
        // (Use workspace shell or check for dirty files)
    }
    if task.contains("report") {
        // Check if last report was recent — if so, skip
    }
    
    // For dev tasks: check if the fix already exists
    // (git log for commit matching task description)
    
    true // Default: cook it
}
```

**~15 LOC. Pre-checks use filesystem/git, not LLM calls — zero token cost.**

### 6. CookState Leak Prevention

Add a timeout to the CookState guard to prevent permanent heartbeat deferral:

```rust
// In heartbeat_loop, instead of just checking is_active():
let cook_active_duration = channel_active.as_ref()
    .map(|s| s.active_since())
    .flatten();

if let Some(since) = cook_active_duration {
    if since.elapsed() > Duration::from_secs(600) {
        warn!("CookState has been active for >10min — forcing clear");
        channel_active.as_ref().map(|s| s.force_clear());
    }
}
```

**~10 LOC. Prevents the heartbeat from being permanently deferred by a leaked CookState.**

## Implementation Plan

| # | Component | LOC | Priority | Depends On |
|---|-----------|-----|----------|------------|
| 1 | Adaptive iteration budget | ~5 | P0 | Nothing |
| 2 | Auto-advance task queue | ~10 | P0 | Nothing |
| 3 | CookState leak prevention | ~10 | P0 | Nothing |
| 4 | LLM pre-check (smart skip) | ~15 | P1 | Nothing |
| 5 | Task completion detection | ~20 | P1 | Auto-advance |
| 6 | Stale task pruning | ~15 | P2 | Completion detection |

**Total: ~75 LOC across heartbeat.rs + cook_state.rs**

## References

- Zeus cooking loop: `crates/zeus-prometheus/src/tool_executor.rs:484-992`
- Zeus heartbeat: `crates/zeus-prometheus/src/heartbeat.rs:856-1077`
- OpenClaw heartbeat: `~/openclaw/src/infra/heartbeat-runner.ts`
- CookState: `crates/zeus-core/src/cook_state.rs`
