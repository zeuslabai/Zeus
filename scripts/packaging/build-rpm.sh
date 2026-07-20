#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — RPM Package Builder (.rpm)
#
# Builds an .rpm package from the compiled zeus binary.
# Requires: rpmbuild (rpm-build package)
#
# Usage:
#   ./scripts/packaging/build-rpm.sh                    # uses target/release/zeus
#   ./scripts/packaging/build-rpm.sh /path/to/zeus      # custom binary path
#   ZEUS_VERSION=1.0.0 ./scripts/packaging/build-rpm.sh # override version
#
# Output: dist/zeus-<version>-1.<arch>.rpm
# ═══════════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Configuration ────────────────────────────────────────────────────────────
VERSION="${ZEUS_VERSION:-0.1.0}"
BINARY="${1:-$REPO_ROOT/target/release/zeus}"

# Detect architecture
MACHINE="$(uname -m)"
case "$MACHINE" in
    x86_64)  ARCH="x86_64" ;;
    aarch64) ARCH="aarch64" ;;
    armv7l)  ARCH="armv7hl" ;;
    *)       ARCH="$MACHINE" ;;
esac

echo "Building zeus ${VERSION} .rpm for ${ARCH}..."

# ── Validate binary exists ───────────────────────────────────────────────────
if [ ! -f "$BINARY" ]; then
    echo "Error: zeus binary not found at $BINARY"
    echo "Run 'cargo build --release --bin zeus' first."
    exit 1
fi

# ── Validate rpmbuild is available ───────────────────────────────────────────
if ! command -v rpmbuild >/dev/null 2>&1; then
    echo "Error: rpmbuild not found. Install rpm-build:"
    echo "  Fedora/RHEL: sudo dnf install rpm-build"
    echo "  openSUSE:    sudo zypper install rpm-build"
    exit 1
fi

# ── Set up RPM build tree ────────────────────────────────────────────────────
RPM_ROOT="$REPO_ROOT/dist/rpmbuild"
rm -rf "$RPM_ROOT"
mkdir -p "$RPM_ROOT"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# ── Copy sources ─────────────────────────────────────────────────────────────
cp "$BINARY" "$RPM_ROOT/SOURCES/zeus"
chmod 755 "$RPM_ROOT/SOURCES/zeus"

# Strip binary if possible
if command -v strip >/dev/null 2>&1; then
    strip "$RPM_ROOT/SOURCES/zeus" 2>/dev/null || true
fi

# Systemd service
cp "$REPO_ROOT/scripts/systemd/zeus-gateway.service" "$RPM_ROOT/SOURCES/"

# Generate shell completions
mkdir -p "$RPM_ROOT/SOURCES/completions"
if "$BINARY" completion bash > "$RPM_ROOT/SOURCES/completions/zeus.bash" 2>/dev/null; then
    echo "  Generated bash completions"
else
    echo "  Warning: could not generate bash completions"
fi

if "$BINARY" completion zsh > "$RPM_ROOT/SOURCES/completions/_zeus" 2>/dev/null; then
    echo "  Generated zsh completions"
else
    echo "  Warning: could not generate zsh completions"
fi

if "$BINARY" completion fish > "$RPM_ROOT/SOURCES/completions/zeus.fish" 2>/dev/null; then
    echo "  Generated fish completions"
else
    echo "  Warning: could not generate fish completions"
fi

# License
if [ -f "$REPO_ROOT/LICENSE" ]; then
    cp "$REPO_ROOT/LICENSE" "$RPM_ROOT/SOURCES/copyright"
elif [ -f "$REPO_ROOT/LICENSE-MIT" ]; then
    cp "$REPO_ROOT/LICENSE-MIT" "$RPM_ROOT/SOURCES/copyright"
else
    cat > "$RPM_ROOT/SOURCES/copyright" << 'LICEOF'
Zeus is dual-licensed under MIT OR Apache-2.0.
See https://github.com/zeuslabai/Zeus for full license text.
LICEOF
fi

# ── Copy spec file ───────────────────────────────────────────────────────────
cp "$SCRIPT_DIR/zeus.spec" "$RPM_ROOT/SPECS/"

# ── Build RPM ────────────────────────────────────────────────────────────────
DIST_DIR="$REPO_ROOT/dist"
mkdir -p "$DIST_DIR"

rpmbuild \
    --define "_topdir $RPM_ROOT" \
    --define "zeus_version $VERSION" \
    --target "$ARCH" \
    -bb "$RPM_ROOT/SPECS/zeus.spec"

# ── Copy output RPM to dist/ ────────────────────────────────────────────────
RPM_FILE=$(find "$RPM_ROOT/RPMS" -name "zeus-*.rpm" -print -quit)
if [ -n "$RPM_FILE" ]; then
    cp "$RPM_FILE" "$DIST_DIR/"
    RPM_BASENAME="$(basename "$RPM_FILE")"
    echo ""
    echo "Built: $DIST_DIR/$RPM_BASENAME"
    echo "Install: sudo rpm -i $DIST_DIR/$RPM_BASENAME"
    echo "    or:  sudo dnf install $DIST_DIR/$RPM_BASENAME"
else
    echo "Error: RPM file not found after build"
    exit 1
fi

# Clean up build tree
rm -rf "$RPM_ROOT"
