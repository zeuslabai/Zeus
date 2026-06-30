# FreeBSD Deployment

Zeus runs on FreeBSD with full functionality except for macOS-specific features (Talos AppleScript automation, Seatbelt sandboxing, iMessage). The application-level security features in Aegis (command filtering, URL allowlisting, approval system, audit logging) remain fully functional.

## Prerequisites

- **Rust toolchain**: Install via [rustup](https://rustup.rs/) or `pkg install rust`.
- **C compiler**: Included in the `lang/rust` port; otherwise `pkg install gcc`.
- **System libraries**: Required for TLS, SQLite, audio, and Opus codec support.

```bash
pkg install -y sqlite3 openssl pkgconf cmake opus
```

> **Optional:** Install `chromium` for browser automation (zeus-browser CDP support).

## Automated Deploy (Recommended)

The `scripts/deploy-freebsd.sh` script handles the full install in one command — builds from source, installs the binary, sets up the rc.d service, deploys the web frontend, generates shell completions, and configures Claude Code MCP.

Run it **on the FreeBSD machine**:

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd Zeus
./scripts/deploy-freebsd.sh
```

### Flags

```
--no-pull      Skip git pull
--no-web       Skip web frontend deploy
--no-service   Skip rc.d service setup
--no-mcp       Skip Claude Code MCP configuration
--cli-only     Only build + install CLI binary
--clean        cargo clean before build
--test         Run cargo test --workspace before build
--force-config Regenerate config.toml and .env (backs up old ones)
--user USER    Zeus service user (default: zeus)
-h, --help     Show help
```

## Building from Source

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd Zeus

cargo build --release
```

The release binary is at `target/release/zeus`.

## Installation

```bash
cp target/release/zeus /usr/local/bin/zeus
```

## rc.d Service

The repository includes a production rc.d script at `scripts/freebsd/zeus-gateway`. The deploy script installs it automatically; to install manually:

```bash
cp scripts/freebsd/zeus-gateway /usr/local/etc/rc.d/zeus_gateway
chmod +x /usr/local/etc/rc.d/zeus_gateway
```

### Enabling the Service

Add to `/etc/rc.conf`:

```sh
zeus_gateway_enable="YES"
zeus_gateway_user="zeus"          # user to run as (default: zeus)
zeus_gateway_port="3001"          # port (default: 3001)
zeus_gateway_host="0.0.0.0"       # bind address
zeus_gateway_logfile="/var/log/zeus-gateway.log"
```

Zeus always loads its config from `~/.zeus/config.toml` (relative to the service user's home directory). Place API keys and secrets in `~/.zeus/.env`.

### Managing the Service

```bash
# Start
service zeus_gateway start

# Stop
service zeus_gateway stop

# Status (no sudo required)
service zeus_gateway status

# Restart
service zeus_gateway restart

# View logs
tail -f /var/log/zeus-gateway.log
```

### Persistent Service (run without login)

The rc.d setup runs at system boot automatically once `zeus_gateway_enable="YES"` is set in `/etc/rc.conf`. No additional configuration is needed.

## Firewall

If running the API server, open the configured port (default 3001):

```bash
# pf (FreeBSD default firewall) — add to /etc/pf.conf:
pass in proto tcp to any port 3001
```

## Platform Notes

- **Talos tools** are not available on FreeBSD. The 193 AppleScript-based macOS automation tools are skipped. All other tools (core tools, browser automation, MCP, etc.) work normally.
- **Seatbelt sandboxing** is not available on FreeBSD. The application-level Aegis security features (command filtering, URL allowlisting, approval system, audit logging) remain fully functional.
- **iMessage** adapter is not available on FreeBSD.
- **Keychain** falls back to file-based credential storage if no compatible secret service is available.
- **Audio tools** require the Opus codec (`pkg install opus`) for voice functionality.
