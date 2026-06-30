---
name: zeus-fleet-deploy
description: Deploy, release, or push updates to the Zeus fleet. Use when deploying to nodes, releasing a build, pushing a new binary, or updating the fleet after a merge.
---

# Zeus Fleet Deploy

## When to Use

Trigger on: deploy, push to fleet, release, update nodes, ship a build, roll out a change.

NOT for: config-only changes (use zeus-config-audit), health checks after a deploy (use zeus-fleet-health), rollbacks (use zeus-rollback).

---

## Procedure

### Step 1 — Pre-Deploy Checklist

Run all three gates. Do not skip any.

```bash
# 1. Run tests
cd ~/Zeus && pnpm test

# 2. Build the binary
./scripts/build.sh

# 3. Validate config on the local node before pushing anywhere
./scripts/config-guard.sh
```

**STOP if any of these fail.** Fix before proceeding. A failed test or broken config-guard means the build is not deployable.

Hard blocks — abort deploy if:
- `model = "anthropic/unknown"` appears in any node's config
- `config-guard.sh` exits non-zero
- `pnpm test` has failures (not just warnings)

---

### Step 2 — Push to Fleet

```bash
# Deploy to all Mac nodes
./scripts/deploy-fleet.sh

# Check deploy status across fleet
./scripts/deploy-fleet.sh --status
```

Nodes: `.100`, `.106`, `.107`, `.112`, `.102`

If `deploy-fleet.sh` is unavailable or fails mid-run, fall back to manual single-node push (see Fallback section below).

---

### Step 3 — Per-Node Verification

For each node that received the deploy, verify:

```bash
# Check gateway process is running
ssh zeus@192.168.1.{NODE} "pgrep -f zeus-gateway && echo OK || echo DEAD"

# Check Discord relay is alive
ssh zeus@192.168.1.{NODE} "pgrep -f zeus-relay && echo OK || echo DEAD"

# Check health endpoint responds
ssh zeus@192.168.1.{NODE} "curl -sf http://localhost:8080/health && echo OK"

# Confirm config-guard passes on the node
ssh zeus@192.168.1.{NODE} "cd ~/Zeus && ./scripts/config-guard.sh"
```

A node is **green** when: gateway running + relay running + health endpoint 200 + config-guard clean.

---

### Step 4 — Smoke Test

At least one agent must respond on Discord after deploy. Send a test message to the fleet channel and wait for a reply. If no response within 2 minutes, treat as failed deploy and begin rollback.

```bash
# Confirm all nodes are on the same commit
./scripts/deploy-fleet.sh --status | grep commit
```

All nodes should show the same git commit hash. Mismatch = incomplete deploy, not a hard failure, but log it.

---

### Step 5 — Post-Deploy Report

Report to Discord channel with:
- Commit hash deployed
- Nodes updated (list)
- Any nodes that failed verification
- Smoke test result (agent responded: yes/no)

Format: `Deploy complete. Commit: {hash}. Nodes: {list}. Smoke test: OK/FAIL.`

---

## Rollback Procedure

If smoke test fails or critical nodes are down:

```bash
# 1. Identify last known-good commit
git log --oneline -10

# 2. Check out the last good commit
git checkout {LAST_GOOD_COMMIT}

# 3. Rebuild
./scripts/build.sh

# 4. Re-deploy
./scripts/deploy-fleet.sh

# 5. Verify again (Step 3 above)
```

Document what failed in the daily memory note before ending the session.

---

## Fallback — Manual Single-Node Deploy

When `deploy-fleet.sh` is unavailable:

```bash
# Build locally
cd ~/Zeus && ./scripts/build.sh

# Copy binary to node
scp ./target/release/zeus-gateway zeus@192.168.1.{NODE}:~/Zeus/target/release/

# Restart gateway on node
ssh zeus@192.168.1.{NODE} "pkill -f zeus-gateway; sleep 2; cd ~/Zeus && ./scripts/start-gateway.sh &"

# Verify
ssh zeus@192.168.1.{NODE} "curl -sf http://localhost:8080/health"
```

Repeat per node. This is slower but reliable.

---

## Quality Gates

- MUST run `pnpm test` before deploy — no exceptions
- MUST run `config-guard.sh` before and after deploy
- MUST verify at least one Discord response after deploy (smoke test)
- MUST NOT deploy with `model = "anthropic/unknown"` in config
- MUST NOT skip rollback if smoke test fails
- MUST report commit hash in post-deploy message

---

## Common Gotchas

**macOS SIP / permissions:** If the binary won't execute after copy, check that it's not quarantined. Run `xattr -d com.apple.quarantine ./zeus-gateway` if needed.

**SSH key not loaded:** Pre-flight: `ssh-add -l` should show your key. If not, `ssh-add ~/.ssh/id_ed25519` before running deploy-fleet.sh.

**Gateway hangs on restart:** Old process sometimes holds the port. Run `lsof -i :8080` on the node and kill any lingering process before starting the new gateway.

**Partial deploy:** If deploy-fleet.sh stops mid-run, nodes may be on different commits. Run `--status` to see which nodes got the update, then manually push to the rest.

**Config path wrong after deploy:** The gateway reads `~/.zeus/config.toml`. If the deploy changes the binary location, verify the config path hasn't shifted. config-guard.sh will catch this.

**Discord relay not reconnecting:** After a gateway restart, the relay may need a separate kick. If the relay process is dead: `ssh zeus@{NODE} "cd ~/Zeus && ./scripts/start-relay.sh &"`.
