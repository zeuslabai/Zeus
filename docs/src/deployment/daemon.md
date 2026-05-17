# Daemon Mode

The `zeus gateway` command runs Zeus as a unified daemon that combines the API server, messaging channel listeners, heartbeat scheduler, and cron task runner into a single long-running process. This is the recommended deployment mode for always-on Zeus installations.

## What the Gateway Runs

When you run `zeus gateway`, four subsystems start:

| Subsystem | Description |
|-----------|-------------|
| **API Server** | HTTP server exposing all 95+ REST API routes, WebSocket streaming, and the OpenAI-compatible chat completions endpoint. |
| **Channel Listeners** | Inbound message listeners for configured channels (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix). Messages received on these channels are routed through the agent loop. |
| **Heartbeat** | Periodic task runner that reads `HEARTBEAT.md` and executes tasks at their configured frequency (daily, weekly, etc.). |
| **Cron Scheduler** | SQLite-persisted cron scheduler for recurring tasks defined programmatically or via the API. |

## Basic Usage

```bash
# Run the full gateway (all subsystems)
zeus gateway

# Custom port
zeus gateway -p 3000

# Minimal mode: API server only, no channels or cron
zeus gateway --no-channels --no-cron
```

## Flags

| Flag | Description |
|------|-------------|
| `-p`, `--port` | HTTP server port (overrides config, default: 3000) |
| `--no-channels` | Disable all messaging channel listeners |
| `--no-cron` | Disable the cron scheduler |
| `--no-heartbeat` | Disable the heartbeat task runner |

Combining `--no-channels --no-cron` gives you a minimal gateway that only runs the API server -- useful for development or when you only need the REST API.

## Configuration

Gateway settings are in the `[api]` section of `~/.zeus/config.toml`:

```toml
[api]
host = "127.0.0.1"    # Listen address (use 0.0.0.0 for all interfaces)
port = 3000            # HTTP port
cors = true            # Enable CORS headers
auth_token = ""        # Bearer token for API authentication (empty = no auth)
```

### Network Binding

By default, the gateway binds to `127.0.0.1` (localhost only). To accept connections from other devices on the network (e.g., the iOS app), change the host:

```toml
[api]
host = "0.0.0.0"
```

### Authentication

Set an `auth_token` to require a Bearer token for all API requests:

```toml
[api]
auth_token = "your-secret-token"
```

Clients must include the header `Authorization: Bearer your-secret-token` with every request.

### Channel Configuration

Each channel requires its own configuration section. See the [Channels](../channels/README.md) documentation for per-channel setup.

## Service Management

### macOS (launchd)

Zeus includes built-in commands for launchd service management:

```bash
# Install the launchd plist (creates ~/Library/LaunchAgents/com.zeus.agent.plist)
zeus daemon install

# Start the daemon
zeus daemon start

# Check status
zeus daemon status

# Stop the daemon
zeus daemon stop
```

The plist runs `zeus gateway` with restart-on-failure. Logs go to `~/.zeus/logs/`.

### Linux (systemd)

Create a systemd user service (see [Linux Deployment](./linux.md) for the full service file):

```bash
systemctl --user enable zeus
systemctl --user start zeus
systemctl --user status zeus
```

### FreeBSD (rc.d)

Use the rc.d script (see [FreeBSD Deployment](./freebsd.md)):

```bash
service zeus start
service zeus status
```

## Graceful Shutdown

The gateway handles `SIGINT` (Ctrl+C) and `SIGTERM` gracefully:

1. Stops accepting new HTTP connections.
2. Sends disconnect signals to all channel listeners.
3. Waits for in-flight requests to complete.
4. Flushes pending audit log entries.
5. Closes the SQLite database connections.
6. Exits cleanly.

## Health Checks

The API server exposes health check endpoints for monitoring:

```bash
# Basic health check
curl http://localhost:3000/health

# Detailed status (model, provider, session count)
curl http://localhost:3000/v1/status

# Full diagnostics (config, workspace, credentials, ollama connectivity)
curl http://localhost:3000/v1/doctor
```

These are useful for load balancer health checks or monitoring systems.

## Logging

Zeus uses structured logging via the `tracing` crate. Log output goes to stderr by default. When running as a daemon, logs are captured by the service manager (launchd, systemd, or rc.d).

Set the log level via the `RUST_LOG` environment variable:

```bash
RUST_LOG=info zeus gateway          # Standard logging
RUST_LOG=debug zeus gateway         # Verbose logging
RUST_LOG=zeus_agent=debug zeus gateway  # Debug logging for agent loop only
```

## Running Behind a Reverse Proxy

For production deployments, place the gateway behind a reverse proxy (nginx, Caddy, etc.) for TLS termination, rate limiting, and additional authentication:

```nginx
server {
    listen 443 ssl;
    server_name zeus.example.com;

    ssl_certificate /etc/ssl/certs/zeus.pem;
    ssl_certificate_key /etc/ssl/private/zeus.key;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

The `proxy_set_header Upgrade` and `Connection "upgrade"` lines are required for WebSocket support (`/v1/ws`).
