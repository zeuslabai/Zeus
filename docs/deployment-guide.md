# Zeus Deployment Guide

Production deployment for Zeus gateway across macOS, Linux, and FreeBSD.

## Architecture Overview

Zeus runs as a single `zeus gateway` process that combines:
- HTTP API server (default port 8080)
- WebSocket streaming
- Channel adapters (Discord, Telegram, Slack, etc.)
- Heartbeat + cron scheduler
- Autonomous orchestration loop

One process, one config file, one binary.

## Prerequisites

- Zeus binary at `/usr/local/bin/zeus`
- Config at `~/.zeus/config.toml`
- Workspace at `~/.zeus/workspace/`
- At least one LLM provider configured (API key or Ollama)

## Quick Deploy

```bash
# Install from source
git clone https://github.com/zeuslabai/Zeus.git && cd Zeus
cargo build --release
sudo cp target/release/zeus /usr/local/bin/

# Or use the installer
./scripts/install.sh

# Run onboarding wizard
zeus onboard

# Start gateway
zeus gateway
```

## Platform-Specific Service Setup

### macOS (launchd)

Zeus installs a launchd plist automatically via `zeus daemon install`:

```bash
zeus daemon install   # Creates ~/Library/LaunchAgents/ai.zeus.gateway.plist
zeus daemon start     # Starts the service
zeus daemon status    # Check if running
zeus daemon stop      # Stop the service
```

**Manual plist** (`~/Library/LaunchAgents/ai.zeus.gateway.plist`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "...">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.zeus.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/zeus</string>
        <string>gateway</string>
        <string>--host</string>
        <string>0.0.0.0</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/zeus-gateway.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/zeus-gateway.log</string>
</dict>
</plist>
```

### Linux (systemd)

Copy the service file:

```bash
sudo cp scripts/systemd/zeus-gateway.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable zeus-gateway
sudo systemctl start zeus-gateway
```

The service file (`scripts/systemd/zeus-gateway.service`) includes security hardening:
- `NoNewPrivileges=yes`
- `ProtectSystem=strict`
- `ProtectHome=read-only`
- `ReadWritePaths=/home/zeus/.zeus`
- `PrivateTmp=yes`

Check logs: `journalctl -u zeus-gateway -f`

### FreeBSD (rc.d)

```bash
sudo cp scripts/freebsd/zeus-gateway /usr/local/etc/rc.d/
sudo chmod +x /usr/local/etc/rc.d/zeus-gateway
```

Add to `/etc/rc.conf`:

```sh
zeus_gateway_enable="YES"
zeus_gateway_port="8080"
zeus_gateway_host="0.0.0.0"
zeus_gateway_user="zeus"
```

```bash
sudo service zeus_gateway start
sudo service zeus_gateway status
```

## Configuration

### Minimal config (`~/.zeus/config.toml`)

```toml
model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20

[gateway]
host = "0.0.0.0"
port = 8080
enable_agent_processing = true
enable_heartbeat = true
```

### Authentication

Set `ZEUS_API_TOKEN` to protect all API endpoints:

```bash
export ZEUS_API_TOKEN=$(openssl rand -hex 32)
```

All clients must send `Authorization: Bearer <token>` (or `?token=<token>` for WebSocket).

### Channel Adapters

Enable messaging channels in config:

```toml
[channels.discord]
token = "your-bot-token"

[channels.telegram]
api_id = 12345
api_hash = "your-hash"
phone = "+1234567890"

[channels.slack]
bot_token = "xoxb-..."
app_token = "xapp-..."
```

See individual channel docs in `docs/howto-*.md`.

## Reverse Proxy

### nginx

```nginx
server {
    listen 443 ssl;
    server_name zeus.example.com;

    ssl_certificate /etc/letsencrypt/live/zeus.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/zeus.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # WebSocket support
    location /v1/ws {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 3600s;
    }
}
```

### Caddy

```
zeus.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy handles TLS and WebSocket upgrades automatically.

## WebUI Deployment

The WebUI is a Leptos/WASM single-page app served by the gateway:

```toml
[gateway]
web_port = 8081          # Separate port for WebUI
web_static_dir = "/opt/zeus/dist"  # Built WASM assets
```

Build the WebUI:

```bash
cd apps/ZeusWeb
trunk build --release    # Outputs to dist/
```

Or use the installer flag: `./scripts/install.sh --with-webui`

## Health Checks

```bash
# Basic health
curl http://localhost:8080/health

# Detailed diagnostics
curl http://localhost:8080/v1/doctor

# Status with model info
curl -H "Authorization: Bearer $ZEUS_API_TOKEN" http://localhost:8080/v1/status
```

For monitoring, poll `/health` every 30s. It returns 200 when the gateway is operational.

## Backup

Critical files to back up:

```
~/.zeus/config.toml          # Configuration (sacred - never lose)
~/.zeus/workspace/           # Memory, prompts, workspace files
~/.zeus/sessions/            # Session history (JSONL)
~/.zeus/mnemosyne.db         # Semantic memory database
```

## Troubleshooting

### Gateway won't start — "Address already in use"

Another instance is running. Check the PID file:

```bash
cat ~/.zeus/gateway.pid
kill $(cat ~/.zeus/gateway.pid)
```

### Config corruption

Zeus creates backups automatically. Restore from known-good:

```bash
cp ~/.zeus/config.toml.known-good ~/.zeus/config.toml
```

Or run the config guard:

```bash
./scripts/config-guard.sh --fix
```

### Channel adapter failures

Check individual channel status:

```bash
curl -H "Authorization: Bearer $ZEUS_API_TOKEN" http://localhost:8080/v1/channels
```

### Memory database locked

If Mnemosyne reports "database is locked", another process has the SQLite file open:

```bash
fuser ~/.zeus/mnemosyne.db    # Linux
lsof ~/.zeus/mnemosyne.db     # macOS
```

## Fleet Deployment

For multi-node deployments, each node runs its own gateway. Coordination happens via:

1. **Discord relay** — agents communicate through a shared Discord channel
2. **Pantheon missions** — structured task dispatch across the fleet
3. **Node registry** — agents discover each other via WebSocket (`/v1/ws/nodes`)

Each node needs:
- Its own `config.toml` with unique agent identity
- Shared Discord channel for fleet communication
- Network access to other nodes' gateway ports (for remote tool execution)

See `CLAUDE.md` for the full fleet roster and coordination patterns.
