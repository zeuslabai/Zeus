# Auto-Detect Task Assignment — Design Document

## Objective
When a coordinator or user assigns a task to an agent via Discord (or any channel), the agent automatically persists it to HEARTBEAT.md and TaskStore (SQLite) without manual editing. This closes the autonomy loop: assign → auto-persist → heartbeat drives work → auto-advance on completion.

## Two-Layer Approach

### Layer A: Prompt-Based (Primary)
The agent's system prompt (AGENTS.md) instructs it to self-persist tasks.

**Already implemented:** The "Task Management" section in AGENTS.md tells agents:
> "When assigned a task, write it to your HEARTBEAT.md under CURRENT TASK immediately."

**Enhancement needed:** The instruction exists but agents don't consistently follow it. Strengthen the prompt to make it a hard rule, not a suggestion.

**Changes:**
1. Update AGENTS.md default template — make task persistence a Core Principle (not just a section)
2. Add explicit examples: "When Zeus100 says 'fix the IRC bug', immediately run write_file on HEARTBEAT.md"
3. Add to the heartbeat prompt: "Before responding to any task assignment, write it to HEARTBEAT.md first"

**Files:** `crates/zeus-memory/src/lib.rs` (DEFAULT_AGENTS template)

**Owner:** Zeus112
**LOC:** ~20 lines of prompt text
**Risk:** Low — prompt change only, no code behavior change

---

### Layer B: Code-Based (Fallback)
The gateway detects task assignments and auto-writes to HEARTBEAT.md + TaskStore when the agent fails to self-persist.

**Trigger conditions (must match ALL):**
1. Inbound message mentions the agent (by name or @mention)
2. Message contains task-like intent (action verbs + technical context)
3. Agent's HEARTBEAT.md CURRENT TASK is empty after the cook completes
4. The agent acknowledged the task in its response ("on it", "working on", "will do")

**Detection logic:**
```rust
fn is_task_assignment(message: &str, agent_name: &str) -> bool {
    let mentions_agent = message.to_lowercase().contains(&agent_name.to_lowercase())
        || message.contains(&format!("@{}", agent_name));
    
    let task_verbs = ["fix", "implement", "build", "add", "create", "ship", 
                      "push", "write", "update", "refactor", "audit", "review",
                      "test", "deploy", "research", "design", "investigate"];
    let has_task_verb = task_verbs.iter().any(|v| message.to_lowercase().contains(v));
    
    let has_branch = message.contains("branch:") || message.contains("Branch:");
    let has_file_ref = message.contains(".rs") || message.contains(".ts") 
        || message.contains("crates/") || message.contains("src/");
    
    mentions_agent && (has_task_verb || has_branch || has_file_ref)
}
```

**Auto-persist flow:**
```
1. Channel message arrives → agent cooks response
2. After cook completes:
   a. Check: was the inbound message a task assignment?
   b. Check: is HEARTBEAT.md CURRENT TASK still empty?
   c. If both: extract task description → write to HEARTBEAT.md + TaskStore
3. Next heartbeat tick picks up the persisted task
```

**Task extraction:**
- Use the agent's response to extract the task description (the agent already understands what was asked)
- Or use a simple heuristic: take the sentence containing the task verb + any branch/file references
- Write as structured CURRENT TASK: description, branch (if mentioned), files (if mentioned)

**Files:**
- `src/gateway.rs` — post-cook task detection + auto-persist hook
- `crates/zeus-memory/src/lib.rs` — `set_current_task()` method on Workspace

**Owner:** Zeus112 (gateway) + ASSISTANT (TaskStore integration)
**LOC:** ~80 lines
**Risk:** Medium — false positives could overwrite real CURRENT TASK. Mitigated by only writing when CURRENT TASK is empty.

---

## Implementation Plan

### Phase 1: Strengthen Prompt (Layer A) — 30 min
- **Zeus112:** Update AGENTS.md template with stronger task persistence instructions
- **Deliverable:** Updated `DEFAULT_AGENTS` in `zeus-memory/src/lib.rs`
- **Branch:** `feat/auto-detect-task-prompt`

### Phase 2: Gateway Fallback (Layer B) — 1-2 hours  
- **Zeus112:** Implement `is_task_assignment()` detection in gateway
- **ASSISTANT:** Wire TaskStore creation on detection
- **zeus106:** Add `set_current_task()` to Workspace (write to HEARTBEAT.md)
- **Branch:** `feat/auto-detect-task-gateway`

### Phase 3: Testing — 30 min
- **Test 1:** Assign task via Discord → verify HEARTBEAT.md updated
- **Test 2:** Assign task when CURRENT TASK is already set → verify no overwrite
- **Test 3:** Non-task message → verify no false positive
- **Test 4:** Task completes → verify advance_task_queue fires

---

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| Agent already has CURRENT TASK | Don't overwrite — add to TASK QUEUE instead |
| Multiple agents mentioned | Each agent persists independently |
| Vague message ("good work") | No task detected — skip |
| Task with branch name | Extract and include in CURRENT TASK |
| Task reassignment ("switch to X") | Clear current, set new |
| Agent fails to respond | Gateway fallback writes task anyway |

---

## Success Criteria
1. merakizzz assigns a task on Discord → agent auto-persists within 1 cook cycle
2. No manual HEARTBEAT.md editing required for 90%+ of assignments
3. Zero false positives on non-task messages (greetings, status checks, etc.)
4. Task survives gateway restart (SQLite + HEARTBEAT.md dual persistence)

---

## Timeline
- Phase 1: Today (prompt update)
- Phase 2: Today (gateway fallback)
- Phase 3: Today (testing)
- Total: ~3 hours across 3 agents
