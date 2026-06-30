# macOS Deployment

Zeus is developed primarily on macOS and has full platform support including Talos (AppleScript-based macOS automation), Seatbelt sandboxing, and native SwiftUI apps.

## Prerequisites

- **Rust toolchain**: Install via [rustup](https://rustup.rs/) (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- **Xcode Command Line Tools**: `xcode-select --install`
- **SQLite3**: Included with macOS by default.

## Building from Source

```bash
# Clone the repository
git clone https://github.com/zeuslabai/Zeus.git
cd zeus

# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test --workspace
```

The release binary is at `target/release/zeus`.

## Installation

Copy the binary to a location on your PATH:

```bash
cp target/release/zeus /usr/local/bin/zeus
```

Or use the install script if available:

```bash
./scripts/install.sh
```

## Launchd Daemon

Zeus can run as a background service managed by launchd. The built-in daemon commands handle plist creation and service management:

```bash
# Install the launchd plist
zeus daemon install

# Start the daemon
zeus daemon start

# Check status
zeus daemon status

# Stop the daemon
zeus daemon stop
```

The `zeus daemon install` command creates a plist file at `~/Library/LaunchAgents/com.zeus.agent.plist` that runs `zeus gateway` with appropriate logging and restart policies.

The daemon starts automatically on login. Logs are written to `~/.zeus/logs/`.

## macOS Desktop App

The Desktop app is a SwiftUI application that uses UniFFI bindings to call into the Rust core directly (no network required).

### Building the Desktop App

1. Build the Rust FFI library:

```bash
./scripts/build-zeus-ffi.sh
```

This compiles `zeus-ffi` as a universal binary (arm64 + x86_64), generates Swift bindings via UniFFI, and packages the result as an XCFramework.

2. Open the Xcode workspace:

```bash
open apps/Zeus.xcworkspace
```

3. Select the **ZeusDesktop** target and build (Cmd+B) or run (Cmd+R).

The Desktop app provides a 3-column layout with Dashboard, Chat, Tools, Memory, Settings, and a MenuBar presence for quick access.

## iOS App

The iOS app is a SwiftUI application that connects to a running Zeus gateway over REST and WebSocket. It does not compile any Rust code -- it is a pure Swift client.

### Running the iOS App

1. Start the Zeus gateway on your Mac:

```bash
zeus gateway
```

2. Open the Xcode workspace:

```bash
open apps/Zeus.xcworkspace
```

3. Select the **ZeusMobile** target, choose a simulator or device, and run.

4. In the iOS app settings, configure the gateway URL (e.g., `http://192.168.1.100:3000` for a local network deployment).

The iOS app provides a TabView with Home, Sessions, Chat, Tools, Memory, and Settings screens.

## Shell Completions

Generate shell completions for your preferred shell:

```bash
zeus completion zsh > ~/.zsh/completions/_zeus
zeus completion bash > /usr/local/etc/bash_completion.d/zeus
zeus completion fish > ~/.config/fish/completions/zeus.fish
```

## Homebrew (Optional)

If a Homebrew formula is available:

```bash
brew install zeus
```

The formula handles building from source and installing the binary, man pages, and shell completions.
