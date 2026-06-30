#!/bin/sh
set -eu

# ══════════════════════════════════════════════════════════════════════════
# Zeus — macOS DMG Builder
# Builds a distributable .dmg containing the Zeus CLI binary + launcher
# ══════════════════════════════════════════════════════════════════════════

VERSION="${1:-0.1.0}"
DMG_NAME="Zeus-${VERSION}-macOS"
APP_NAME="Zeus"
BUILD_DIR="$(mktemp -d)"
STAGING="${BUILD_DIR}/${DMG_NAME}"

cleanup() { rm -rf "$BUILD_DIR"; }
trap cleanup EXIT

echo "Building Zeus ${VERSION} for macOS..."

# ── 1. Build release binary ──────────────────────────────────────────────

if [ ! -f "target/release/zeus" ]; then
    echo "Building release binary..."
    cargo build --release --bin zeus
fi

BINARY="target/release/zeus"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Release binary not found at $BINARY"
    exit 1
fi

# Verify it's a universal or native binary
ARCH=$(file "$BINARY" | grep -o 'arm64\|x86_64' | head -1)
echo "Binary architecture: ${ARCH:-unknown}"

# ── 2. Create staging directory ──────────────────────────────────────────

mkdir -p "$STAGING"

# Binary
cp "$BINARY" "$STAGING/zeus"
chmod 755 "$STAGING/zeus"

# Install helper script
cat > "$STAGING/install.sh" << 'INSTALL_EOF'
#!/bin/sh
set -eu
DEST="/usr/local/bin"
echo "Installing Zeus to $DEST..."
if [ ! -w "$DEST" ]; then
    sudo cp ./zeus "$DEST/zeus"
    sudo chmod 755 "$DEST/zeus"
else
    cp ./zeus "$DEST/zeus"
    chmod 755 "$DEST/zeus"
fi
echo "Zeus installed. Run 'zeus onboard' to get started."
echo "Run 'zeus doctor' to verify your installation."
INSTALL_EOF
chmod 755 "$STAGING/install.sh"

# README
cat > "$STAGING/README.txt" << 'README_EOF'
Zeus — Autonomous AI Assistant
==============================

Quick Start:
  1. Double-click install.sh (or run ./install.sh in Terminal)
  2. Run: zeus onboard
  3. Run: zeus (launches TUI)

Commands:
  zeus               Launch TUI
  zeus serve          Run API server
  zeus gateway        Run unified daemon
  zeus chat "hello"   Quick message
  zeus doctor         Check installation
  zeus onboard        Setup wizard

Website: https://zeuslab.ai
GitHub:  https://github.com/zeuslabai/Zeus
README_EOF

# Shell completions (generate if binary works)
if "$BINARY" completion bash > /dev/null 2>&1; then
    mkdir -p "$STAGING/completions"
    "$BINARY" completion bash > "$STAGING/completions/zeus.bash" 2>/dev/null || true
    "$BINARY" completion zsh > "$STAGING/completions/_zeus" 2>/dev/null || true
    "$BINARY" completion fish > "$STAGING/completions/zeus.fish" 2>/dev/null || true
fi

# ── 3. Create DMG ────────────────────────────────────────────────────────

DMG_PATH="${DMG_NAME}.dmg"
echo "Creating DMG: ${DMG_PATH}..."

# Use hdiutil to create a compressed DMG
hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$STAGING" \
    -ov \
    -format UDZO \
    "$DMG_PATH"

# Get file size
SIZE=$(du -h "$DMG_PATH" | cut -f1)
echo ""
echo "DMG created: ${DMG_PATH} (${SIZE})"
echo "SHA256: $(shasum -a 256 "$DMG_PATH" | cut -d' ' -f1)"
