# Deployment — System Services

Deploy Zeus as a persistent system service that starts on boot, auto-restarts on crash, and logs properly.

## macOS (launchd)

### Automatic Installation

```bash
zeus daemon install
zeus daemon start
zeus daemon status
```

This creates a launchd plist at `~/Library/LaunchAgents/com.zeus.gateway.plist`.

### Manual Installation

Create the plist file:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
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
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/youruser</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/youruser/.zeus/gateway.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/youruser/.zeus/gateway.err</string>
</dict>
</plist>
```

Load and start:

```bash
launchctl load ~/Library/LaunchAgents/com.zeus.gateway.plist
launchctl start com.zeus.gateway
```

### Management

```bash
zeus daemon start       # Start the service
zeus daemon stop        # Stop the service
zeus daemon status      # Check status
zeus daemon install     # Install plist
```

Or directly:

```bash
launchctl list | grep zeus
launchctl stop com.zeus.gateway
launchctl start com.zeus.gateway
```

## FreeBSD (rc.d)

### Install the Service

```bash
sudo cp deployment/deploy-freebsd.sh /usr/local/etc/rc.d/zeus_gateway
sudo chmod +x /usr/local/etc/rc.d/zeus_gateway
```

Or use the deployment script:

```bash
# On the FreeBSD machine:
deployment/deploy-freebsd.sh
```

### Configure

```bash
# Enable on boot
sudo sysrc zeus_gateway_enable=YES

# Set the user
sudo sysrc zeus_gateway_user=mike

# Set the port
sudo sysrc zeus_gateway_port=3001
```

### Config File

Place configuration at `/usr/local/etc/zeus/config.toml`.

Environment variables at `/usr/local/etc/zeus/.env`:

```bash
ANTHROPIC_API_KEY=sk-ant-...
```

### Management

```bash
sudo service zeus_gateway start
sudo service zeus_gateway stop
sudo service zeus_gateway status
sudo service zeus_gateway restart
```

### Logs

```bash
tail -f /var/log/zeus-gateway.log
```

## Linux (systemd)

### Create Service Unit

```ini
# /etc/systemd/user/zeus-gateway.service
[Unit]
Description=Zeus Gateway
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/zeus gateway --host 0.0.0.0 --port 3001
Restart=always
RestartSec=5
EnvironmentFile=%h/.zeus/.env
WorkingDirectory=%h

[Install]
WantedBy=default.target
```

### Enable and Start

```bash
systemctl --user daemon-reload
systemctl --user enable zeus-gateway
systemctl --user start zeus-gateway
systemctl --user status zeus-gateway
```

### Logs

```bash
journalctl --user -u zeus-gateway -f
```

## FreeBSD-Specific Notes

- Install `pkg install rust llvm openssl sqlite3 ffmpeg` before building
- Set `ulimit -n 10240` before launching (or add to rc.d script)
- `cpal` (audio) doesn't work on FreeBSD — Zeus falls back gracefully
- Piper TTS binary not in FreeBSD packages — TTS falls back to system speech

## Deployment Checklist

- [ ] Binary built with `--release` (never deploy debug builds — 282MB vs 99MB)
- [ ] Config at the expected path (`~/.zeus/config.toml` or `/usr/local/etc/zeus/config.toml`)
- [ ] Environment variables set (`.env` file readable by the service user)
- [ ] Port not blocked by firewall
- [ ] Service set to auto-start on boot
- [ ] Logs rotated (logrotate or newsyslog)
- [ ] Health check: `curl http://localhost:3001/health`

## What's Next

→ [[12-Gateway]] — Gateway configuration and operation
→ [[15-Security]] — Security settings for production
