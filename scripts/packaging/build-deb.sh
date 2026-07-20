#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Debian Package Builder (.deb)
#
# Builds a .deb package from the compiled zeus binary.
# Requires: dpkg-deb, strip (from binutils)
#
# Usage:
#   ./scripts/packaging/build-deb.sh                    # uses target/release/zeus
#   ./scripts/packaging/build-deb.sh /path/to/zeus      # custom binary path
#   ZEUS_VERSION=1.0.0 ./scripts/packaging/build-deb.sh # override version
#
# Output: dist/zeus_<version>_<arch>.deb
# ═══════════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Configuration ────────────────────────────────────────────────────────────
VERSION="${ZEUS_VERSION:-0.1.0}"
BINARY="${1:-$REPO_ROOT/target/release/zeus}"

# Detect architecture
MACHINE="$(uname -m)"
case "$MACHINE" in
    x86_64)  ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
    armv7l)  ARCH="armhf" ;;
    *)       ARCH="$MACHINE" ;;
esac

PKG_NAME="zeus_${VERSION}_${ARCH}"
DIST_DIR="$REPO_ROOT/dist"
STAGE="$DIST_DIR/$PKG_NAME"

echo "Building zeus ${VERSION} .deb for ${ARCH}..."

# ── Validate binary exists ───────────────────────────────────────────────────
if [ ! -f "$BINARY" ]; then
    echo "Error: zeus binary not found at $BINARY"
    echo "Run 'cargo build --release --bin zeus' first."
    exit 1
fi

# ── Clean + create staging directory ─────────────────────────────────────────
rm -rf "$STAGE"
mkdir -p "$STAGE/DEBIAN"
mkdir -p "$STAGE/usr/local/bin"
mkdir -p "$STAGE/etc/bash_completion.d"
mkdir -p "$STAGE/usr/share/zsh/site-functions"
mkdir -p "$STAGE/usr/share/fish/vendor_completions.d"
mkdir -p "$STAGE/usr/lib/systemd/system"
mkdir -p "$STAGE/usr/share/doc/zeus"

# ── Copy binary ──────────────────────────────────────────────────────────────
cp "$BINARY" "$STAGE/usr/local/bin/zeus"
chmod 755 "$STAGE/usr/local/bin/zeus"

# Strip if not already stripped
if command -v strip >/dev/null 2>&1; then
    strip "$STAGE/usr/local/bin/zeus" 2>/dev/null || true
fi

# ── Generate shell completions ───────────────────────────────────────────────
if "$BINARY" completion bash > "$STAGE/etc/bash_completion.d/zeus" 2>/dev/null; then
    echo "  Generated bash completions"
else
    echo "  Warning: could not generate bash completions"
    rm -f "$STAGE/etc/bash_completion.d/zeus"
fi

if "$BINARY" completion zsh > "$STAGE/usr/share/zsh/site-functions/_zeus" 2>/dev/null; then
    echo "  Generated zsh completions"
else
    echo "  Warning: could not generate zsh completions"
    rm -f "$STAGE/usr/share/zsh/site-functions/_zeus"
fi

if "$BINARY" completion fish > "$STAGE/usr/share/fish/vendor_completions.d/zeus.fish" 2>/dev/null; then
    echo "  Generated fish completions"
else
    echo "  Warning: could not generate fish completions"
    rm -f "$STAGE/usr/share/fish/vendor_completions.d/zeus.fish"
fi

# ── Systemd service file ────────────────────────────────────────────────────
cp "$REPO_ROOT/scripts/systemd/zeus-gateway.service" "$STAGE/usr/lib/systemd/system/"

# ── License ──────────────────────────────────────────────────────────────────
if [ -f "$REPO_ROOT/LICENSE" ]; then
    cp "$REPO_ROOT/LICENSE" "$STAGE/usr/share/doc/zeus/copyright"
elif [ -f "$REPO_ROOT/LICENSE-MIT" ]; then
    cp "$REPO_ROOT/LICENSE-MIT" "$STAGE/usr/share/doc/zeus/copyright"
else
    cat > "$STAGE/usr/share/doc/zeus/copyright" << 'LICEOF'
Zeus is dual-licensed under MIT OR Apache-2.0.
See https://github.com/zeuslabai/Zeus for full license text.
LICEOF
fi

# ── Calculate installed size (in KB) ─────────────────────────────────────────
INSTALLED_SIZE=$(du -sk "$STAGE" | cut -f1)

# ── Debian control file ─────────────────────────────────────────────────────
cat > "$STAGE/DEBIAN/control" << EOF
Package: zeus
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Installed-Size: ${INSTALLED_SIZE}
Depends: libc6 (>= 2.31), libssl3 | libssl1.1, libsqlite3-0
Recommends: curl, ca-certificates
Maintainer: Zeus Team <team@zeuslab.ai>
Homepage: https://zeuslab.ai
Description: Autonomous AI assistant with 327 tools and 11 LLM providers
 Zeus is a local-first AI assistant featuring a cognitive engine (Nous),
 multi-channel chat (Telegram, Discord, Slack, Email, iMessage, WhatsApp,
 Signal, Matrix), 5 frontends (TUI, Web, macOS, iOS, visionOS), browser
 automation, voice calls, and security sandboxing.
 .
 Run 'zeus' for the TUI, 'zeus gateway' for the API server,
 or 'zeus setup' for guided first-run configuration.
EOF

# ── Post-install script ──────────────────────────────────────────────────────
cat > "$STAGE/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e

# Create zeus system user if it doesn't exist (for gateway service)
if ! getent group zeus >/dev/null 2>&1; then
    groupadd --system zeus
fi
if ! getent passwd zeus >/dev/null 2>&1; then
    useradd --system --gid zeus --create-home --home-dir /home/zeus zeus
fi

# Create zeus data directory
mkdir -p /home/zeus/.zeus
chown -R zeus:zeus /home/zeus/.zeus

# Reload systemd if available
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload || true
fi

echo ""
echo "Zeus installed successfully!"
echo ""
echo "  Quick start:    zeus"
echo "  Setup wizard:   zeus setup"
echo "  API gateway:    zeus gateway"
echo "  Start service:  systemctl enable --now zeus-gateway"
echo ""
echo "  Config:     ~/.zeus/config.toml"
echo "  Secrets:    ~/.zeus/.env"
echo ""
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

# ── Pre-remove script ────────────────────────────────────────────────────────
cat > "$STAGE/DEBIAN/prerm" << 'EOF'
#!/bin/sh
set -e

# Stop service if running
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop zeus-gateway 2>/dev/null || true
    systemctl disable zeus-gateway 2>/dev/null || true
fi
EOF
chmod 755 "$STAGE/DEBIAN/prerm"

# ── Build the .deb ──────────────────────────────────────────────────────────
mkdir -p "$DIST_DIR"
dpkg-deb --build --root-owner-group "$STAGE" "$DIST_DIR/${PKG_NAME}.deb"

echo ""
echo "Built: $DIST_DIR/${PKG_NAME}.deb"
echo "Install: sudo dpkg -i $DIST_DIR/${PKG_NAME}.deb"

# Clean up staging
rm -rf "$STAGE"
