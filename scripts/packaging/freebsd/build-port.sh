# Zeus — FreeBSD Port Build Script
#
# Creates a FreeBSD .txz package from the pre-built zeus binary.
# Requires: pkg (base system on FreeBSD; install via pkg on Linux)
#
# Usage:
#   ./scripts/packaging/freebsd/build-port.sh                 # uses target/release/zeus
#   ./scripts/packaging/freebsd/build-port.sh /path/to/zeus   # custom binary path
#   ZEUS_VERSION=1.0.0 ./build-port.sh                        # override version
#
# Output: dist/zeus-<version>.txz
# ═══════════════════════════════════════════════════════════════════════════════

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Configuration ───────────────────────────────────────────────────────────
VERSION="${ZEUS_VERSION:-0.1.0}"
BINARY="${1:-$REPO_ROOT/target/release/zeus}"
PKG_NAME="zeus"
DIST_DIR="$REPO_ROOT/dist"

# FreeBSD package naming
PKG_FILE="${DIST_DIR}/${PKG_NAME}-${VERSION}.txz"

echo "Building zeus ${VERSION} .txz package for FreeBSD..."

# ── Validate binary exists ───────────────────────────────────────────────────
if [ ! -f "$BINARY" ]; then
    echo "Error: zeus binary not found at $BINARY"
    echo "Run 'cargo build --release --bin zeus' first."
    exit 1
fi

# ── Set up staging tree ──────────────────────────────────────────────────────
STAGE="$(mktemp -d)"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

STAGE_ROOT="$STAGE/zeus-${VERSION}"
mkdir -p "$STAGE_ROOT/usr/local/bin"
mkdir -p "$STAGE_ROOT/usr/local/man/man1"
mkdir -p "$STAGE_ROOT/usr/local/etc/zeus"
mkdir -p "$STAGE_ROOT/usr/local/share/examples/zeus"
mkdir -p "$STAGE_ROOT/var/db/pkg"

# ── Copy binary ──────────────────────────────────────────────────────────────
cp "$BINARY" "$STAGE_ROOT/usr/local/bin/zeus"
chmod 755 "$STAGE_ROOT/usr/local/bin/zeus"

# Strip binary if possible (reduces size)
if command -v strip >/dev/null 2>&1; then
    strip "$STAGE_ROOT/usr/local/bin/zeus" 2>/dev/null || true
fi

# ── Generate man page from --help ────────────────────────────────────────────
if command -v groff >/dev/null 2>&1; then
    "$BINARY" --help 2>&1 | groff -man -T utf8 > "$STAGE_ROOT/usr/local/man/man1/zeus.1" 2>/dev/null || true
elif command -v mandoc >/dev/null 2>&1; then
    "$BINARY" --help 2>&1 | mandoc -m 1 > "$STAGE_ROOT/usr/local/man/man1/zeus.1" 2>/dev/null || true
fi

# ── Shell completions ────────────────────────────────────────────────────────
mkdir -p "$STAGE_ROOT/usr/local/share/bash-completion/completions"
mkdir -p "$STAGE_ROOT/usr/local/share/zsh/site-functions"
mkdir -p "$STAGE_ROOT/usr/local/share/fish/vendor_completions.d"

"$BINARY" completion bash > "$STAGE_ROOT/usr/local/share/bash-completion/completions/zeus" 2>/dev/null || true
"$BINARY" completion zsh > "$STAGE_ROOT/usr/local/share/zsh/site-functions/_zeus" 2>/dev/null || true
"$BINARY" completion fish > "$STAGE_ROOT/usr/local/share/fish/vendor_completions.d/zeus.fish" 2>/dev/null || true

# ── Config sample ────────────────────────────────────────────────────────────
if [ -f "$REPO_ROOT/scripts/packaging/freebsd/config.toml.sample" ]; then
    cp "$REPO_ROOT/scripts/packaging/freebsd/config.toml.sample" "$STAGE_ROOT/usr/local/etc/zeus/config.toml.sample"
fi

# ── License ──────────────────────────────────────────────────────────────────
mkdir -p "$STAGE_ROOT/usr/local/share/doc/zeus"
if [ -f "$REPO_ROOT/LICENSE" ]; then
    cp "$REPO_ROOT/LICENSE" "$STAGE_ROOT/usr/local/share/doc/zeus/"
elif [ -f "$REPO_ROOT/LICENSE-MIT" ]; then
    cp "$REPO_ROOT/LICENSE-MIT" "$STAGE_ROOT/usr/local/share/doc/zeus/"
fi

# ── Build the .txz package ───────────────────────────────────────────────────
mkdir -p "$DIST_DIR"

if command -v pkg >/dev/null 2>&1; then
    # FreeBSD native: use pkg create
    pkg create \
        -o "$DIST_DIR" \
        -p "$SCRIPT_DIR/pkg-plist" \
        -r "$STAGE_ROOT" \
        -m "$SCRIPT_DIR" \
        -t txz
    PKG_FILE="${DIST_DIR}/${PKG_NAME}-${VERSION}.txz"
else
    # Linux fallback: use tar + xz (users can convert with 'pkg' tool)
    tar -C "$STAGE_ROOT" -cvf "$STAGE/${PKG_NAME}-${VERSION}.tar" .
    xz -9 "$STAGE/${PKG_NAME}-${VERSION}.tar"
    PKG_FILE="${DIST_DIR}/${PKG_NAME}-${VERSION}.txz"
    mv "$STAGE/${PKG_NAME}-${VERSION}.tar.xz" "$PKG_FILE"
fi

if [ -f "$PKG_FILE" ]; then
    echo ""
    echo "Built: $PKG_FILE"
    echo "Install: sudo pkg install $PKG_FILE"
    echo "    or:  pkg add $PKG_FILE"
    echo ""
    echo "On FreeBSD, zeus will be available at /usr/local/bin/zeus"
    echo "Config: ~/.zeus/config.toml"
else
    echo "Error: package file not found after build"
    exit 1
fi