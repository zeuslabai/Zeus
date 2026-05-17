# Installing Zeus

Zeus is a single binary. Pick the method that suits you.

---

## Homebrew (macOS / Linux) — recommended

```bash
brew tap zeuslabai/zeus
brew install zeus
```

That's it. The formula installs the binary, generates shell completions, and initializes `~/.zeus/` on first install.

### Update

```bash
brew upgrade zeus
```

---

## Install Script

```bash
curl -sSL https://raw.githubusercontent.com/zeuslabai/Zeus/main/scripts/install.sh | bash
```

Downloads the correct pre-built binary for your platform, places it in `/usr/local/bin/`, and optionally configures Zeus as an MCP server for Claude Code.

---

## Pre-built Binary (manual)

Download from the [Releases page](https://github.com/zeuslabai/Zeus/releases/latest):

| Platform | Binary |
|----------|--------|
| macOS Apple Silicon | `zeus-1.0.0-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `zeus-1.0.0-x86_64-apple-darwin.tar.gz` |
| Linux ARM64 | `zeus-1.0.0-aarch64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 | `zeus-1.0.0-x86_64-unknown-linux-gnu.tar.gz` |

```bash
tar -xzf zeus-1.0.0-aarch64-apple-darwin.tar.gz
sudo mv zeus /usr/local/bin/
zeus --version
```

---

## Build from Source

Requires Rust 1.85+ (2024 edition). Install via [rustup](https://rustup.rs/).

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd zeus
cargo build --release
sudo cp target/release/zeus /usr/local/bin/
```

Build time: ~3–5 minutes on Apple Silicon, ~8–10 minutes on Intel.

---

## FreeBSD

Zeus ships as a static binary — no shared lib dependencies.

```bash
# Download the Linux x86_64 binary or build from source:
git clone https://github.com/zeuslabai/Zeus.git
cd zeus
cargo build --release
sudo cp target/release/zeus /usr/local/bin/

# Install the rc.d service script
sudo cp scripts/freebsd/zeus /usr/local/etc/rc.d/
sudo chmod +x /usr/local/etc/rc.d/zeus
sudo sysrc zeus_enable="YES"
sudo service zeus start
```

---

## First Run

After installing, run the interactive setup wizard:

```bash
zeus onboard
```

This walks through:
1. LLM provider selection (Anthropic, OpenAI, Ollama, and 8 more)
2. API key entry or OAuth login
3. Model selection (live-fetched from your provider)
4. Channel setup (Telegram, Discord, Slack, and 5 more — all optional)
5. Security level
6. Skills selection
7. Launch options

Or skip straight to the TUI:

```bash
zeus
```

Zeus will prompt for the minimum config it needs on first launch.

---

## Verify

```bash
zeus --version      # Should print: zeus 1.0.0
zeus doctor         # Runs 17 diagnostic checks
```

---

## Uninstall

### Homebrew

```bash
brew uninstall zeus
brew untap zeuslabai/zeus
```

### Manual

```bash
sudo rm /usr/local/bin/zeus
rm -rf ~/.zeus          # removes config, sessions, workspace — irreversible
```

---

## System Requirements

| | Minimum | Recommended |
|---|---------|-------------|
| **OS** | macOS 14+ or Linux (glibc 2.31+) | macOS 15+ |
| **RAM** | 256 MB | 512 MB+ |
| **Disk** | 50 MB (binary) | 500 MB (with sessions/memory) |
| **Network** | Required for LLM API calls | — |
| **Rust** | 1.85+ (build from source only) | Latest stable |

For macOS Talos automation tools (193 AppleScript tools), macOS 14 Sonoma or later is required.

---

## Troubleshooting

**`zeus: command not found`**
- Homebrew: make sure `/opt/homebrew/bin` is in your `$PATH`
- Manual install: check `/usr/local/bin` is in your `$PATH`

**`zeus doctor` shows API key warnings**
- Set your key: `export ANTHROPIC_API_KEY="sk-ant-..."` or add it to `~/.zeus/config.toml`
- Run `zeus onboard` to reconfigure

**Permission denied on macOS**
- Gatekeeper may block unsigned binaries: `xattr -d com.apple.quarantine /usr/local/bin/zeus`

**Gateway won't start**
- Check `~/.zeus/config.toml` exists and has a valid `model` setting
- Run `zeus doctor` for a full diagnostic

**TUI shows garbled characters**
- Ensure your terminal uses a UTF-8 locale: `export LANG=en_US.UTF-8`
- Use a Nerd Font or any monospace font with Unicode support
