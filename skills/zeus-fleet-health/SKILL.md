---
name: zeus-fleet-health
description: Check the health of the Zeus fleet. Use for morning standups, post-deploy verification, node status checks, or when an agent appears to be down or unresponsive.
---

# Zeus Fleet Health

## When to Use

Trigger on: fleet health, node status, morning standup, after deploy, agent down, unresponsive node, is everyone up.

NOT for: deploying updates (use zeus-fleet-deploy), fixing config issues (use zeus-config-audit), debugging heartbeat noise (use zeus-heartbeat-debug).

---

## Fleet Roster

| Node | IP | Agent |
|------|----|-------|
| zeus100 | 192.168.1.100 | Coordinator |
| zeus106 | 192.168.1.106 | Herald |
| zeus107 | 192.168.1.107 | Herald |
| zeus112 | 192.168.1.112 | TBD |
| zeus102 | 192.168.1.102 | TBD |

---

## Procedure

### Step 1 — Reachability Sweep

Ping all nodes first. Fast — tells you immediately who's offline.

```bash
for ip in 192.168.1.100 192.168.1.106 192.168.1.107 192.168.1.112 192.168.1.102; do
  ping -c 1 -W 1 $ip > /dev/null 2>&1 && echo "$ip: REACHABLE" || echo "$ip: UNREACHABLE"
done
```

Unreachable nodes: log as RED immediately. Skip SSH checks for them.

---

### Step 2 — Per-Node Health Check

For each reachable node:

```bash
NODE=192.168.1.{X}

# 1. Gateway process running?
ssh zeus@$NODE "pgrep -f zeus-gateway && echo GATEWAY:OK || echo GATEWAY:DEAD"

# 2. Health endpoint responding?
ssh zeus@$NODE "curl -sf http://localhost:8080/health && echo HEALTH:OK || echo HEALTH:FAIL"

# 3. Discord relay running?
ssh zeus@$NODE "pgrep -f zeus-relay && echo RELAY:OK || echo RELAY:DEAD"

# 4. Config valid?
ssh zeus@$NODE "cd ~/Zeus && ./scripts/config-guard.sh && echo CONFIG:OK || echo CONFIG:FAIL"

# 5. Workspace path exists?
ssh zeus@$NODE "WORKSPACE=\$(grep 'workspace' ~/.zeus/config.toml | head -1 | cut -d'\"' -f2); test -d \"\$WORKSPACE\" && echo WORKSPACE:OK || echo WORKSPACE:MISSING"

# 6. Disk space (warn if >85% full)
ssh zeus@$NODE "df -h ~ | awk 'NR==2{print \"DISK:\"\$5}'"

# 7. Uptime
ssh zeus@$NODE "uptime | awk '{print \"UPTIME:\"\$3\" \"\$4}'"

# 8. Current git commit
ssh zeus@$NODE "cd ~/Zeus && git rev-parse --short HEAD 2>/dev/null || echo COMMIT:UNKNOWN"
```

---

### Step 3 — Hung Process Detection

A node with a running gateway but no heartbeat activity in >30min is hung, not healthy.

```bash
# Check last heartbeat log timestamp
ssh zeus@$NODE "ls -lt ~/.zeus/logs/ | head -5"

# Check last agent activity in gateway logs
ssh zeus@$NODE "tail -20 ~/.zeus/logs/gateway.log | grep -E 'heartbeat|activity|iteration'"
```

If last activity timestamp is >30 minutes ago and gateway is running: flag as YELLOW (hung/stuck, not dead).

---

### Step 4 — Version Alignment Check

All nodes should be on the same git commit. Mismatch means a partial deploy or a node that missed an update.

```bash
for ip in 192.168.1.100 192.168.1.106 192.168.1.107 192.168.1.112 192.168.1.102; do
  COMMIT=$(ssh zeus@$ip "cd ~/Zeus && git rev-parse --short HEAD 2>/dev/null || echo UNKNOWN")
  echo "$ip: $COMMIT"
done
```

If nodes are split across commits: flag which nodes are behind. This is YELLOW, not RED, unless the behind-nodes are actively misbehaving.

---

### Step 5 — Generate Status Matrix

Compile all results into a readable table:

```
Node     | IP              | Gateway | Health | Relay | Config | Disk  | Status
---------|-----------------|---------|--------|-------|--------|-------|-------
zeus100  | 192.168.1.100   | ✅      | ✅     | ✅    | ✅     | 42%   | 🟢 GREEN
zeus106  | 192.168.1.106   | ✅      | ✅     | ✅    | ✅     | 67%   | 🟢 GREEN
zeus107  | 192.168.1.107   | ✅      | ❌     | ✅    | ✅     | 71%   | 🟡 YELLOW
zeus112  | 192.168.1.112   | ❌      | ❌     | ❌    | ❌     | —     | 🔴 RED
zeus102  | 192.168.1.102   | ✅      | ✅     | ✅    | ⚠️     | 89%   | 🟡 YELLOW
```

Post to Discord if doing a morning standup or post-deploy check.

---

## Status Definitions

**🟢 GREEN** — All checks pass. Node is fully operational.

**🟡 YELLOW** — One or more non-critical issues:
- Health endpoint down but gateway running
- Disk >85% full
- Behind on git commit
- Last activity >30min ago (possible hung process)
- Config has warnings (not hard failures)

**🔴 RED** — Critical failure:
- Node unreachable
- Gateway process dead
- Config corruption detected
- Relay dead + no Discord activity

---

## Alert Escalation

| Condition | Action |
|-----------|--------|
| 1 node RED | Log it, attempt restart, report to coordinator |
| 2+ nodes RED | Page coordinator immediately, do not attempt solo recovery |
| Coordinator (.100) RED | Escalate to human owner — this is the command node |
| Disk >90% | Alert immediately — logs will stop writing, sessions will break |
| All nodes on different commits | Flag as deployment incident, run zeus-fleet-deploy to re-align |

---

## Quick Diagnostic — Single Node

When one node is acting weird and you need a fast diagnosis:

```bash
NODE=192.168.1.{X}

echo "=== $NODE DIAGNOSTIC ==="
ssh zeus@$NODE "
  echo '--- Process Status ---'
  pgrep -a zeus-gateway
  pgrep -a zeus-relay
  echo '--- Health Endpoint ---'
  curl -sf http://localhost:8080/health || echo 'ENDPOINT DEAD'
  echo '--- Config ---'
  cat ~/.zeus/config.toml | grep -E 'model|workspace|channel_id' | head -10
  echo '--- Last 10 Log Lines ---'
  tail -10 ~/.zeus/logs/gateway.log 2>/dev/null || echo 'NO LOG FILE'
  echo '--- Disk ---'
  df -h ~
  echo '--- Git ---'
  cd ~/Zeus && git log --oneline -3
"
```

---

## Quality Gates

- MUST check all 5 nodes — never skip a node without noting it as unreachable
- MUST detect hung processes (gateway running but no activity >30min)
- MUST verify all nodes are on the same git commit
- MUST flag disk usage >85% as YELLOW, >90% as RED
- MUST generate status matrix before reporting
- MUST escalate to coordinator if 2+ nodes are RED
- MUST treat coordinator node (.100) as highest priority

---

## Common Gotchas

**SSH timeout slows the sweep:** Add `-o ConnectTimeout=5` to SSH commands when doing the full sweep to avoid waiting 30s per unreachable node.

**Gateway running but health endpoint dead:** This usually means the gateway started but crashed mid-init. Check `tail -50 ~/.zeus/logs/gateway.log` for panic or error output.

**config-guard.sh not found:** Means the Zeus repo hasn't been updated on that node. It's probably behind on commits. Run `git pull` on that node as a first step.

**Relay appears dead but Discord messages are going through:** The relay may be running under a different process name. Check `ps aux | grep zeus` to see what's actually running.

**Disk check shows wrong partition:** `df -h ~` may show the home partition, not the system partition. If logs are on a different mount, check that separately: `df -h ~/.zeus/`.

**Node on different timezone:** Uptime and log timestamps may look off compared to coordinator. Always use UTC for cross-node time comparisons: `ssh zeus@$NODE "date -u"`.

**False GREEN from cached SSH connection:** If you have ControlMaster enabled in SSH config, a cached connection may return stale results. Force a fresh connection with `-o ControlMaster=no` if results seem wrong.
