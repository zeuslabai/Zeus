# Installation

## Prerequisites

Zeus is written in Rust and requires the following to build:

- **Rust toolchain** (1.75 or later) -- Install via [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **System libraries** -- Required for TLS, SQLite, audio, and Opus codec. On macOS these are provided by the system or Homebrew. On Linux:
  ```bash
  # Debian / Ubuntu
  sudo apt install -y build-essential pkg-config \
    libssl-dev libsqlite3-dev \
    libasound2-dev libdbus-1-dev \
    cmake libopus-dev

  # Fedora / RHEL
  sudo dnf install -y gcc make pkg-config sqlite-devel openssl-devel cmake opus-devel

  # Arch Linux
  sudo pacman -S base-devel sqlite openssl pkg-config cmake opus
  ```

## Building from Source

Clone the repository and build a release binary:

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd zeus
cargo build --release
```

The compiled binary is located at `target/release/zeus`. You can copy it to a directory on your `PATH`:

```bash
cp target/release/zeus /usr/local/bin/
```

To verify the build:

```bash
zeus doctor
```

## Running the Test Suite

To confirm everything is working correctly, run the full test suite:

```bash
cargo test --workspace
```

This executes all 1,711 tests across the 20 workspace crates.

## Optional Dependencies

These are not required for basic operation but enable additional capabilities:

- **Ollama** -- Local LLM inference and embedding generation (used by Mnemosyne for vector search with `nomic-embed-text`). Install from [ollama.com](https://ollama.com/).
- **Google Chrome / Chromium** -- Required for browser automation via the Chrome DevTools Protocol. Zeus launches Chrome in headless or visible mode as needed.
- **Twilio account** -- Required for voice call functionality. Set `TWILIO_ACCOUNT_SID`, `TWILIO_AUTH_TOKEN`, and `TWILIO_PHONE_NUMBER` environment variables.
- **signal-cli** -- Required for the Signal messaging adapter. Must be available on `PATH`.

## macOS Desktop App

The native macOS desktop app requires additional setup:

1. Build the Rust FFI library:
   ```bash
   ./scripts/build-zeus-ffi.sh
   ```
2. Open the Xcode workspace:
   ```bash
   open apps/Zeus.xcworkspace
   ```

The desktop app uses UniFFI bindings to call into the Rust core.

## iOS App

The iOS app connects to a running Zeus gateway over the network -- no Rust compilation is needed on the device. Build it by opening `apps/ZeusMobile/` in Xcode and running on a simulator or device. Make sure the gateway is accessible at the configured URL.
