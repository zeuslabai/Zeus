---
name: zeus-config-audit
description: Audit and repair ~/.zeus/config.toml on any node. Use when validating config, diagnosing node crashes, investigating config corruption, or running pre-deploy checks.
---

# zeus-config-audit

## When to Use

Trigger on: "config audit", "validate config", "config corruption", "node won't start", "why did it crash", "config looks wrong", pre-deploy checks.

Do NOT use for: general shell debugging, git issues, Discord API problems unrelated to config.

---

## Procedure

### Step 1 — Read the Config

```bash
cat ~/.zeus/config.toml
```

If the file is missing:
```bash
ls -la ~/.zeus/
```
→ If `~/.zeus/` doesn't exist, the node has never been initialized. Stop and report: **node uninitialized**.

---

### Step 2 — Validate Required Sections

Check that all of the following sections and keys are present:

**[agent]**
- `name` — must be set (e.g. `zeus106`)
- `model` — must NOT be `"anthropic/unknown"` → flag immediately
- `workspace` — path must exist on disk and must NOT be a `/var/folders/` temp path

**[gateway]**
- `url` — must be present

**[oauth]**
- `token` — must be present if `use_oauth = true` in [agent]
- Credentials must be in `[oauth]`, NOT in a `[credentials]` section

**[bindings]** (or equivalent Discord config)
- `discord_token` — must be present
- `channel_id` — must be set

```bash
# Quick key-presence checks
grep -E "model|workspace|use_oauth|discord_token|channel_id" ~/.zeus/config.toml
```

---

### Step 3 — Run Targeted Checks

**3a. Temp path check (critical — past corruption root cause)**
```bash
grep "workspace" ~/.zeus/config.toml | grep "/var/folders"
```
→ Any match = CRITICAL. The workspace is in a temp dir that gets wiped. Generate repair command (see Step 5).

**3b. Unknown model check**
```bash
grep 'model' ~/.zeus/config.toml | grep "unknown"
```
→ Any match = CRITICAL. Agent will fail all API calls.

**3c. OAuth placement check**
```bash
# Credentials should be in [oauth], not [credentials]
grep -A5 "\[credentials\]" ~/.zeus/config.toml
```
→ If `[credentials]` section exists with a token, flag: **credentials in wrong section (pre-S78 format)**.

**3d. Stale credentials.json check**
```bash
ls ~/.zeus/credentials.json 2>/dev/null && echo "FOUND — should not exist post-S78"
```
→ If found: flag for removal.

**3e. Config file permissions**
```bash
stat -f "%Sp %N" ~/.zeus/config.toml   # macOS
# stat -c "%a %n" ~/.zeus/config.toml  # Linux
```
→ Must be `0600` (`-rw-------`). If world-readable: flag as security risk.

**3f. Workspace path existence**
```bash
WORKSPACE=$(grep "workspace" ~/.zeus/config.toml | head -1 | sed 's/.*= *"//' | sed 's/".*//')
ls -la "$WORKSPACE" 2>/dev/null || echo "WORKSPACE MISSING: $WORKSPACE"
```

**3g. Sessions path existence**
```bash
SESSIONS=$(grep "sessions" ~/.zeus/config.toml | head -1 | sed 's/.*= *"//' | sed 's/".*//')
ls -la "$SESSIONS" 2>/dev/null || echo "SESSIONS PATH MISSING: $SESSIONS"
```

---

### Step 4 — Generate Audit Report

Output a structured summary:

```
CONFIG AUDIT — <node_name> — <timestamp>

[CRITICAL]
  ❌ model = "anthropic/unknown"         → Fix: update model to valid provider/model
  ❌ workspace in /var/folders/...       → Fix: reset to ~/Zeus/workspace

[WARNING]
  ⚠️  credentials.json exists            → Fix: rm ~/.zeus/credentials.json
  ⚠️  config.toml permissions: 0644     → Fix: chmod 600 ~/.zeus/config.toml

[OK]
  ✅ discord_token present
  ✅ channel_id present
  ✅ oauth token in [oauth] section
  ✅ sessions path exists

STATUS: NEEDS REPAIR / CLEAN
```

---

### Step 5 — Generate Repair Commands

For each issue found, output the exact repair command. Never auto-apply — always show first.

```bash
# Fix: model unknown
sed -i '' 's/model = "anthropic\/unknown"/model = "anthropic\/claude-sonnet-4-6"/' ~/.zeus/config.toml

# Fix: temp workspace path
sed -i '' 's|workspace = "/var/folders/.*"|workspace = "'"$HOME/Zeus/workspace"'"|' ~/.zeus/config.toml

# Fix: stale credentials.json
rm ~/.zeus/credentials.json

# Fix: config permissions
chmod 600 ~/.zeus/config.toml
```

After showing repair commands, ask: "Apply these? (y/n)" — or if running autonomously in a deploy pipeline, apply and log.

---

### Step 6 — Re-verify After Repair

After any repair, re-run steps 2–3 to confirm the issue is resolved. Report clean or escalate.

---

## Quality Gates

- **MUST** detect `/var/folders` temp paths — this was the root cause of past corruption
- **MUST** flag `model = "anthropic/unknown"` — agent will be non-functional
- **MUST** verify OAuth token is in `[oauth]` section, not `[credentials]`
- **MUST NOT** auto-apply repairs without showing them first (unless in automated pipeline)
- **MUST** re-verify after any repair

---

## Common Gotchas

**macOS `stat` vs Linux `stat`**
`stat -f` is macOS syntax. On Linux nodes use `stat -c "%a %n"`. Check with `uname -s`.

**`sed -i` on macOS requires empty string argument**
`sed -i ''` on macOS, `sed -i` on Linux. Get OS first:
```bash
OS=$(uname -s)
if [ "$OS" = "Darwin" ]; then SED_I="sed -i ''"; else SED_I="sed -i"; fi
```

**Config may be TOML with inline tables**
Some values may be inline: `model = { provider = "anthropic", name = "claude-sonnet-4-6" }`. Parse accordingly — `grep "unknown"` still catches it.

**Symlinked config**
If `~/.zeus/config.toml` is a symlink, `chmod` won't work on the link target unless you resolve it:
```bash
readlink -f ~/.zeus/config.toml
```

**`[credentials]` section may be vestigial from pre-S78**
If the section exists but `[oauth]` also has a valid token, the system may still work. Flag it as cleanup debt but don't mark CRITICAL — verify actual auth method with `grep "use_oauth" ~/.zeus/config.toml`.

---

## Scope

This skill covers `~/.zeus/config.toml` validation only.

For full fleet-wide config checks across all nodes, use `zeus-fleet-health`.
For deploy pipeline config validation, this skill is called as a sub-step by `zeus-fleet-deploy`.
