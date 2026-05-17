# Heartbeat-Driven Task Queue — Autonomy Design

## Problem
Agents idle for 7+ hours despite having assigned tasks. The heartbeat fires every 5 minutes but doesn't drive real work. Agents only cook when a human sends a Discord/Telegram message.

## Root Cause
1. `workspace.get_heartbeat_tasks()` is called in `heartbeat.rs` but **the method doesn't exist** on the Workspace struct — task extraction from HEARTBEAT.md was never implemented
2. HEARTBEAT.md has a `CURRENT TASK` slot but it contains generic placeholder text, not specific backlog items
3. The heartbeat uses "light context mode" — it runs with a stripped system prompt and basic task descriptions, not the full agent context needed for coding
4. After a cook completes, there's no mechanism to start the next cook — the agent waits for the next inbound message

## Current Flow (broken)
```
Discord message → gateway → agent.run() → cook → respond → IDLE
                                                               ↑
Heartbeat (5min) → read HEARTBEAT.md → generic task → HEARTBEAT_OK → IDLE
```

## Proposed Flow (autonomous)
```
Discord message → gateway → agent.run() → cook → respond → check queue
                                                               ↓
                                                          next task? → cook → respond → check queue
                                                               ↓ (no)
                                                          IDLE (clean)
                                                               ↑
Heartbeat (5min) → read task queue → pending task? → inject as message → cook → mark done → check queue
```

## Design: Task Queue System

### 1. HEARTBEAT.md Format (enhanced)
```markdown
## CURRENT TASK
Implement onboarding Manual mode differentiation
Branch: feat/onboarding-manual-mode
Files: crates/zeus-tui/src/onboarding/mod.rs
Assigned by: Zeus100
Priority: P1

## TASK QUEUE
- [ ] Make workspace/sessions paths editable in Step 12 | P1 | Branch: feat/onboarding-editable-paths
- [ ] Add Browser/Talos/MCP ability toggles to Step 14 | P2 | Branch: feat/onboarding-ability-toggles
- [ ] Signal typing indicators | P2 | Branch: feat/signal-typing

## COMPLETED
- [x] rustls-tls migration (2026-04-17)
- [x] Microcompact preserve current turn (2026-04-17)
- [x] Tool argument compaction removal (2026-04-17)
```

### 2. Workspace::get_heartbeat_tasks() — Implementation
```rust
pub async fn get_heartbeat_tasks(&self, _frequency: &str) -> Result<Vec<String>> {
    let content = self.read("HEARTBEAT.md").await?;
    let mut tasks = Vec::new();
    
    // Extract CURRENT TASK
    if let Some(current) = extract_section(&content, "CURRENT TASK") {
        if !current.contains("(Coordinator will assign")
           && !current.trim().is_empty() {
            tasks.push(current);
        }
    }
    
    // Extract TASK QUEUE items (unchecked only)
    if let Some(queue) = extract_section(&content, "TASK QUEUE") {
        for line in queue.lines() {
            if line.starts_with("- [ ]") {
                tasks.push(line[5..].trim().to_string());
            }
        }
    }
    
    Ok(tasks)
}
```

### 3. Heartbeat Behavior Change
Current: `timeout_secs: 300` (5 min) — too short for coding tasks
Proposed: When CURRENT TASK has content, use full cooking timeout (1800s) with tools enabled

Current: Light context (SOUL + IDENTITY only)
Proposed: Full agent context when CURRENT TASK is a coding task

### 4. Cook-to-Queue Bridge
After each cook completes in the gateway:
1. Check if the response indicates task completion ("done", "shipped", "pushed")
2. If complete: update HEARTBEAT.md — move CURRENT TASK to COMPLETED, pop TASK QUEUE
3. If not complete: leave CURRENT TASK as-is for next heartbeat
4. If TASK QUEUE has items: immediately start next cook (no 5min wait)

### 5. Standing Orders (OpenClaw pattern)
Persistent tasks that re-fire on schedule:
```markdown
## STANDING ORDERS
- Every 1h: Check git status, report uncommitted work
- Every 30m: Check Discord for unread coordinator messages
- Every 4h: Run cargo test --workspace, report failures
```

### 6. Stop Conditions
- Empty TASK QUEUE + empty CURRENT TASK → HEARTBEAT_OK (clean idle)
- Explicit "PAUSE" in HEARTBEAT.md → skip cooking
- 3+ consecutive cook failures → pause and report to coordinator
- Quiet hours (23:00-08:00) → defer to next active window

## Implementation Plan
1. **Phase 1:** Implement `Workspace::get_heartbeat_tasks()` — parse HEARTBEAT.md
2. **Phase 2:** Wire heartbeat to use full cooking (tools, full context) for CURRENT TASK
3. **Phase 3:** Add cook-to-queue bridge in gateway — auto-advance after completion
4. **Phase 4:** Add standing orders support
5. **Phase 5:** Coordinator tools — Zeus100 can update any agent's HEARTBEAT.md remotely

## OpenClaw Comparison
| Feature | OpenClaw | Zeus (proposed) |
|---------|----------|-----------------|
| Task source | Standing orders file | HEARTBEAT.md TASK QUEUE |
| Execution loop | `while(true)` + sleep | Heartbeat interval + immediate chain |
| Task completion | Mark in file | Move to COMPLETED section |
| Coordinator control | Edit standing orders | Update HEARTBEAT.md via message tool |
| Stop condition | Empty orders | Empty queue + PAUSE flag |

## Priority
This is THE critical path to launch. Without autonomous task execution between cooks, agents are expensive status reporters, not workers.
