---
name: checkpoint
description: "Create a git checkpoint: stash changes or commit WIP, log current state. Safe save point for experimentation."
user-invocable: true
skillKey: checkpoint
read_when:
  - "checkpoint"
  - "save point"
  - "save progress"
---

# Checkpoint — Git Save Point

Create a safe save point before risky changes or experimentation.

## When to Use

- Before attempting a risky refactor
- Before trying an alternative approach
- At the end of a work session
- Before running destructive operations
- When you want to save current progress

## How It Works

### Quick Checkpoint (stash)
```bash
# Save current changes without committing
git stash push -m "checkpoint: [description]"
```

### WIP Checkpoint (commit)
```bash
# Commit work-in-progress
git add -A
git commit -m "WIP: [description]"
```

### Named Branch Checkpoint
```bash
# Create a checkpoint branch
git checkout -b checkpoint/[name]
git add -A
git commit -m "checkpoint: [description]"
git checkout -  # Return to previous branch
```

### Restore
```bash
# Restore from stash
git stash pop

# Restore from WIP commit
git reset --soft HEAD~1

# Restore from branch
git cherry-pick checkpoint/[name]
```

## Rules

- Always include a descriptive message
- Prefer stash for quick experiments (< 30 min)
- Prefer WIP commit for longer work
- Prefer branch checkpoint before major refactors
- Clean up old checkpoints periodically

## Integration

- Use `/checkpoint` before risky changes
- Use `/plan` to design the approach
- Use `/verify` to validate after restoring
