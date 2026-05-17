# Fleet Management — Multi-Machine Agent Coordination

Zeus can run as a fleet of agents across multiple machines. Each agent registers with a gateway, reports heartbeats, and can be assigned to Pantheon missions based on capabilities.

## Architecture

```
┌────────────────┐     ┌────────────────┐     ┌────────────────┐
│  zeus112 (.112)│     │  zeus107 (.107)│     │  fbsd1 (.224)  │
│  macOS M1 Pro  │     │  macOS M2      │     │  FreeBSD 15    │
│  Backend, Docs │     │  Security, TUI │     │  Deploy, CI    │
└───────┬────────┘     └───────┬────────┘     └───────┬────────┘
        │                      │                      │
        └──────────┬───────────┴──────────────────────┘
                   │
          ┌────────▼────────┐
          │  Gateway (.226) │
          │  FreeBSD, :3001 │
          │  API + Relay    │
          └─────────────────┘
```

Each agent:
- Runs a Zeus gateway (or gateway-connected agent)
- Registers itself with capabilities
- Sends heartbeats every few minutes
- Gets assigned to Pantheon mission teams

## Agent Registration

### Auto-Registration on Boot

The gateway calls `boot_fleet_agents()` on startup, which registers known agents from configuration:

```toml
# In config.toml
[[fleet.agents]]
id = "zeus-112"
name = "Zeus112"
host = "192.168.1.112"
capabilities = ["backend", "docs", "orchestration", "rust"]

[[fleet.agents]]
id = "zeus-107"
name = "Zeus107"
host = "192.168.1.107"
capabilities = ["security", "tui", "testing"]
```

### Self-Registration

Remote agents can register themselves:

```bash
curl -X POST http://gateway:3001/v1/fleet/register \
  -H "Content-Type: application/json" \
  -d '{
    "id": "zeus-new",
    "name": "NewAgent",
    "host": "192.168.1.200",
    "capabilities": ["frontend", "css", "react"],
    "model": "claude-sonnet-4-20250514",
    "status": "online"
  }'
```

### Heartbeat

Agents must send heartbeats to stay in the active pool:

```bash
curl -X POST http://gateway:3001/v1/fleet/zeus-112/heartbeat
```

Agents that miss heartbeats for >10 minutes are marked `Offline` and won't be assigned to new missions.

## Listing Fleet Agents

```bash
curl http://gateway:3001/v1/fleet | jq
```

Response:
```json
[
  {
    "id": "zeus-112",
    "name": "Zeus112",
    "host": "192.168.1.112",
    "status": "online",
    "capabilities": ["backend", "docs", "orchestration", "rust"],
    "model": "claude-opus-4-6",
    "last_heartbeat": "2026-07-15T12:34:56Z"
  },
  {
    "id": "zeus-107",
    "name": "Zeus107",
    "host": "192.168.1.107",
    "status": "online",
    "capabilities": ["security", "tui", "testing"],
    "model": "claude-sonnet-4-20250514",
    "last_heartbeat": "2026-07-15T12:33:12Z"
  }
]
```

## Deregistering an Agent

```bash
curl -X DELETE http://gateway:3001/v1/fleet/zeus-old
```

## Capability-Based Team Assembly

When a Pantheon mission is created, the orchestrator selects agents based on capabilities:

1. Goal is decomposed into tasks (e.g., "write backend", "write tests", "write docs")
2. Each task has required capabilities derived from the goal
3. Agents are matched: `backend` capability → backend tasks, `testing` → test tasks
4. Team is assembled from best-matching online agents

### Capability Matching Example

```
Mission: "Build a REST API with tests and documentation"

Tasks:
  1. Design API schema         → needs: [backend, architecture]
  2. Implement endpoints       → needs: [backend, rust]
  3. Write integration tests   → needs: [testing, rust]
  4. Write API documentation   → needs: [docs, technical-writing]

Team assembled:
  - Zeus112 (backend, docs, rust) → Tasks 1, 2, 4
  - Zeus107 (security, testing)   → Task 3
```

## Stale Agent Cleanup

The gateway runs `cleanup_stale_agents()` on boot and periodically:

- Checks `last_heartbeat` for all registered agents
- Agents idle >10 minutes → marked `Offline`
- Offline agents are excluded from team assembly
- Agents come back online when they send their next heartbeat

## Deploying Across Machines

### macOS Agent

```bash
# Build
cd Zeus && cargo build --release

# Install
sudo cp target/release/zeus /usr/local/bin/zeus
codesign --force --sign - /usr/local/bin/zeus  # Required after sudo cp

# Run as service
zeus daemon install
zeus daemon start
```

### FreeBSD Agent

```bash
# Use the deploy script
./deployment/deploy-freebsd.sh

# Or manually
cargo build --release
sudo cp target/release/zeus /usr/local/bin/zeus

# Enable the rc.d service
sudo sysrc zeus_gateway_enable=YES
sudo service zeus_gateway start
```

### Service Configuration

**macOS** (`~/Library/LaunchAgents/com.zeus.gateway.plist`):
```xml
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.zeus.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/zeus</string>
        <string>gateway</string>
        <string>--host</string>
        <string>0.0.0.0</string>
        <string>--port</string>
        <string>3001</string>
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
```

**FreeBSD** (`/usr/local/etc/rc.d/zeus_gateway`):
```sh
#!/bin/sh
. /etc/rc.subr
name="zeus_gateway"
rcvar="zeus_gateway_enable"
command="/usr/local/bin/zeus"
command_args="gateway --host 0.0.0.0 --port 3001"
pidfile="/var/run/${name}.pid"
load_rc_config $name
run_rc_command "$1"
```

## Discord Coordination

Fleet agents communicate via a shared Discord channel. Each agent runs a Discord relay that:
- Receives messages from Discord → forwards to the agent
- Agent responses → posted back to Discord
- War Room messages bridge to the same channel

Set in `~/.zeus/.env`:
```bash
DISCORD_BOT_TOKEN="your-bot-token"
DISCORD_RELAY_CHANNEL_IDS="1475583517156180018"
```

## Important Rules

- **NEVER deploy debug builds** — they're 282MB vs 99MB release, can OOM constrained machines
- **ALWAYS use `cargo build --release`** for deployments
- **ALWAYS `codesign --force --sign -`** after `sudo cp` on macOS (ad-hoc signature gets invalidated)
- **Gateway must be a proper OS service** — not `nohup`, not spawned from MCP
- **NEVER change the binary path** (`/usr/local/bin/zeus`) on failure — diagnose the root cause instead

## What's Next

→ [[16-Deployment]] — System service setup
→ [[13-Pantheon]] — Multi-agent missions
→ [[12-Gateway]] — Gateway configuration
