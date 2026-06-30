---
name: evolve
description: "Cluster related learned patterns into skills, commands, or agent personas. Review confidence scores and promote high-confidence patterns."
user-invocable: true
skillKey: evolve
read_when:
  - "evolve"
  - "cluster patterns"
  - "promote pattern"
---

# Evolve — Pattern Clustering & Promotion

Review learned patterns in Mnemosyne, cluster related ones, and evolve them into reusable skills or conventions.

## When to Use

- After accumulating multiple `/learn` entries
- Periodic review of learned patterns
- When similar patterns appear across sessions
- To promote tentative patterns to established conventions

## How It Works

### Step 1: Review Current Patterns
Search Mnemosyne for all learned patterns:
- List by confidence level (Tentative → Established → Confident → Instinct)
- Group by domain (code-style, testing, debugging, security, workflow)

### Step 2: Cluster Related Patterns
Find patterns that share:
- Same trigger condition
- Same domain/category
- Complementary actions (e.g., "validate input" + "sanitize output")

### Step 3: Evolve Clusters
Transform clusters into:
- **Skill** — A reusable workflow (like `/tdd` or `/verify`)
- **Convention** — A rule added to MEMORY.md or workspace rules
- **Agent persona** — A specialized behavior profile

### Step 4: Promote High-Confidence
Patterns with confidence ≥ 0.8 across multiple sessions → promote to:
- MEMORY.md standing rules
- Workspace rules files
- Skill SKILL.md files

### Confidence Scoring

| Level | Score | Meaning |
|-------|-------|---------|
| Tentative | 0.3 | Observed once, not yet confirmed |
| Moderate | 0.5 | Observed multiple times |
| Established | 0.7 | Consistently confirmed |
| Confident | 0.85 | Core behavior |
| Instinct | 0.95 | Automatic, never questioned |

### Promotion Criteria
- Pattern observed in 3+ sessions
- Confidence ≥ 0.8
- No contradicting evidence
- Actionable and specific

## Integration

- Use `/learn` to capture patterns
- Use `/evolve` periodically to review and promote
- Updated patterns feed into zeus-nous confidence engine
