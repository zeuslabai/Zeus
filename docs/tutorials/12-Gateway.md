# Gateway — Production Daemon

The gateway is Zeus's production mode. It combines the API server, messaging channels, heartbeat, and cron scheduler into a single process.

## Start the Gateway

```bash
zeus gateway
```

This starts:
1. **API Server** — REST + WebSocket on port 3001
2. **Channel Adapters** — Telegram, Discord, Slack, etc. (all configured channels)
3. **Heartbeat** — Proactive processing loop
4. **Cron Scheduler** — Scheduled tasks

## Configuration

```toml
# ~/.zeus/config.toml
[gateway]
host = "127.0.0.1"         # Listen address
port = 3001                 # HTTP port
public_url = "https://gt.zeuslab.ai"  # Public URL for fleet agents
enable_channels = true
enable_cron = true
enable_heartbeat = true
enable_api = true
```

### Listen on All Interfaces

For remote access (e.g., from mobile apps or other machines):

```bash
zeus gateway --host 0.0.0.0 --port 3001
```

## Selectively Disable Subsystems

```bash
zeus gateway --no-channels      # API + cron only (no messaging)
zeus gateway --no-cron          # API + channels only (no scheduled tasks)
zeus gateway --no-channels --no-cron   # API only
```

## What the Gateway Does

### On Startup

1. Loads configuration from `~/.zeus/config.toml`
2. Initializes all subsystems (Mnemosyne, Nous, Aegis, etc.)
3. Starts API server on configured host:port
4. Connects all configured channel adapters
5. Registers fleet agents via `boot_fleet_agents()`
6. Recovers stale missions via `recover_stale_missions()`
7. Cleans up stale agents via `cleanup_stale_agents()`
8. Starts heartbeat loop and cron scheduler

### During Operation

- **Incoming messages** from any channel → routed to the agent loop → response sent back
- **API requests** → handled by Axum REST/WebSocket handlers
- **Heartbeat** → reads `HEARTBEAT.md` for proactive tasks, processes them
- **Cron** → executes scheduled jobs (content queue drain, etc.)
- **Mission timeout check** → every 60 seconds, auto-fails timed-out missions

### Graceful Shutdown

`SIGINT` (Ctrl+C) or `SIGTERM` triggers graceful shutdown:
1. Stops accepting new connections
2. Drains in-progress requests
3. Disconnects channel adapters
4. Persists state to SQLite

## Environment Variables

The gateway reads from `~/.zeus/.env`. For system services, ensure the env file is sourced:

```bash
# Manual start with env
export $(grep -v '^#' ~/.zeus/.env | xargs) && zeus gateway

# Or set in the service file (see Deployment tutorial)
```

## Health Monitoring

```bash
# Quick health check
curl http://localhost:3001/health

# Detailed status
curl http://localhost:3001/v1/status | jq

# Full diagnostics
curl http://localhost:3001/v1/doctor | jq
```

## Logs

Gateway logs to stdout/stderr. Redirect for persistence:

```bash
zeus gateway > ~/.zeus/gateway.log 2>&1
```

Or use a system service (see [[16-Deployment]]) for proper log management.

## What's Next

→ [[13-Pantheon]] — Multi-agent missions
→ [[16-Deployment]] — Install as a system service (launchd/rc.d/systemd)
→ [[09-Channels]] — Configure messaging channels
