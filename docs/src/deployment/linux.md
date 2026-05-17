# Linux Deployment

Zeus runs on Linux with full functionality except for macOS-specific features (Talos AppleScript automation, Seatbelt sandboxing, iMessage). On Linux, Aegis uses seccomp-bpf for sandboxing instead of Seatbelt.

## Prerequisites

- **Rust toolchain**: Install via [rustup](https://rustup.rs/)
- **SQLite3 development libraries**: Required for Mnemosyne's SQLite backend.
- **OpenSSL development libraries**: Required for TLS connections.
- **pkg-config**: Required for locating system libraries.

### Debian / Ubuntu

```bash
sudo apt update
sudo apt install -y \
  build-essential pkg-config \
  libssl-dev libsqlite3-dev \
  libasound2-dev libdbus-1-dev \
  cmake libopus-dev
```

> **Optional:** Install `chromium` for browser automation (zeus-browser CDP support).

### Fedora / RHEL

```bash
sudo dnf install -y gcc make pkg-config sqlite-devel openssl-devel
```

### Arch Linux

```bash
sudo pacman -S base-devel sqlite openssl pkg-config
```

## Building from Source

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd zeus

cargo build --release
```

The release binary is at `target/release/zeus`.

## Installation

```bash
sudo cp target/release/zeus /usr/local/bin/zeus
```

## systemd Service

Create a systemd user service to run Zeus as a background daemon.

### Service File

Create `~/.config/systemd/user/zeus.service`:

```ini
[Unit]
Description=Zeus AI Assistant Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/zeus gateway
Restart=on-failure
RestartSec=10
Environment=HOME=%h
WorkingDirectory=%h

# Optional: set environment variables for API keys
# Environment=ANTHROPIC_API_KEY=sk-ant-...
# Environment=OPENAI_API_KEY=sk-...

# Logging
StandardOutput=journal
StandardError=journal
SyslogIdentifier=zeus

[Install]
WantedBy=default.target
```

### Managing the Service

```bash
# Reload systemd after creating/modifying the service file
systemctl --user daemon-reload

# Enable the service (start on login)
systemctl --user enable zeus

# Start the service
systemctl --user start zeus

# Check status
systemctl --user status zeus

# View logs
journalctl --user -u zeus -f

# Stop the service
systemctl --user stop zeus
```

### Persistent User Services

To allow the service to run even when you are not logged in (e.g., on a server):

```bash
sudo loginctl enable-linger $(whoami)
```

## System-Wide Installation

For a system-wide daemon (running as a dedicated user):

```bash
# Create a dedicated user
sudo useradd -r -s /bin/false -d /var/lib/zeus zeus
sudo mkdir -p /var/lib/zeus/.zeus
sudo chown -R zeus:zeus /var/lib/zeus

# Create system service
sudo cp zeus.service /etc/systemd/system/zeus.service
```

Adjust the service file to run as the `zeus` user:

```ini
[Service]
User=zeus
Group=zeus
WorkingDirectory=/var/lib/zeus
Environment=HOME=/var/lib/zeus
```

## Firewall

If running the API server, open the configured port (default 3000):

```bash
# UFW (Ubuntu)
sudo ufw allow 3000/tcp

# firewalld (Fedora/RHEL)
sudo firewall-cmd --add-port=3000/tcp --permanent
sudo firewall-cmd --reload
```

## Platform Notes

- **Talos tools** are not available on Linux. The 193 AppleScript-based macOS automation tools are skipped. All other tools (core tools, browser automation, MCP, etc.) work normally.
- **seccomp sandboxing** replaces Seatbelt. Aegis automatically uses the Linux-native seccomp-bpf backend when running on Linux.
- **Keychain** uses Linux Secret Service (via D-Bus) for credential storage instead of macOS Keychain.
- **iMessage** adapter is not available on Linux.
