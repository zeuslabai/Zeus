# Deployment

Zeus can be deployed on macOS, Linux, and FreeBSD. The core binary is a single Rust executable that includes the TUI, CLI, API server, and gateway daemon. Native apps (macOS Desktop and iOS) are built separately.

## Supported Platforms

| Platform | Build | Daemon | Talos (macOS Automation) | Native Apps |
|----------|-------|--------|--------------------------|-------------|
| **macOS** (arm64, x86_64) | Full support | launchd | Yes (AppleScript) | Desktop + iOS |
| **Linux** (x86_64, aarch64) | Full support | systemd | No | No |
| **FreeBSD** (x86_64) | Full support | rc.d | No | No |

## Deployment Modes

Zeus supports several deployment modes depending on your needs:

### Interactive (TUI)

The default mode. Launch with `zeus` or `zeus tui` for a full terminal interface with 10 screens.

### Single Message (CLI)

For scripting and automation: `zeus chat "message"` sends a single message and prints the response.

### API Server

`zeus serve` starts an HTTP server exposing the full REST API (95+ routes). Useful for integrating Zeus with other applications.

### Gateway Daemon

`zeus gateway` runs a unified daemon that combines the API server, messaging channel listeners, heartbeat scheduler, and cron tasks. This is the recommended mode for always-on deployments.

## Platform Guides

- [macOS](./macos.md) -- Building from source, launchd daemon, Desktop and iOS apps.
- [Linux](./linux.md) -- Building from source, systemd service, dependencies.
- [FreeBSD](./freebsd.md) -- Building from source, rc.d service, platform notes.
- [Daemon Mode](./daemon.md) -- Detailed guide to the gateway daemon, its flags, and service management.
