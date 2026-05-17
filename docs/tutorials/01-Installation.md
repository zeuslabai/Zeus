# Installation

This tutorial covers three ways to install Zeus: from source, via the install script, or via Homebrew.

## Option 1: Build from Source (Recommended)

Building from source gives you the latest code and all features.

### Step 1 — Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup --version   # verify: 1.86+
```

### Step 2 — Install System Dependencies

**macOS** (most are built-in):
```bash
# Xcode command line tools (if not installed)
xcode-select --install

# Optional: ffmpeg for voice message conversion
brew install ffmpeg
```

**Linux (Debian/Ubuntu)**:
```bash
sudo apt install -y build-essential pkg-config \
  libssl-dev libsqlite3-dev \
  libasound2-dev libdbus-1-dev \
  cmake libopus-dev
```

**FreeBSD**:
```bash
sudo pkg install rust llvm openssl sqlite3 ffmpeg
```

### Step 3 — Clone and Build

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd Zeus
cargo build --release
```

> ⏱ First build takes 5–10 minutes (compiling 31 crates, ~342K lines).

### Step 4 — Install the Binary

```bash
sudo cp target/release/zeus /usr/local/bin/
zeus --version
```

### Step 5 — Verify

```bash
zeus doctor
```

`zeus doctor` runs 17 diagnostic checks: config, workspace, credentials, Ollama connectivity, session health, and more. All green = you're ready.

## Option 2: Install Script

One-liner that downloads (or builds) the binary:

```bash
curl -sSL https://raw.githubusercontent.com/zeuslabai/Zeus/main/scripts/install.sh | bash
```

The script:
1. Detects your OS and architecture
2. Builds from source (clones the repo, runs `cargo build --release`)
3. Copies the binary to `/usr/local/bin/`
4. Optionally configures Zeus as an MCP server for Claude Code

## Option 3: Homebrew (macOS)

```bash
brew tap zeuslabai/zeus
brew install zeus
```

## Updating

```bash
cd Zeus
git pull origin main
cargo build --release
sudo cp target/release/zeus /usr/local/bin/
```

> ⚠️ **NEVER deploy debug builds** to `/usr/local/bin/`. Debug binaries are 282MB vs 99MB release, and will OOM on constrained machines. Always use `cargo build --release`.

> ⚠️ **macOS codesign**: If you `sudo cp` the binary, the ad-hoc signature is invalidated. Fix with: `codesign --force --sign - /usr/local/bin/zeus`

## What's Next

→ [[02-First-Run]] — Run the setup wizard and send your first message
