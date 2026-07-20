#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — nfpm Package Builder (.deb + .rpm from any host, no native tooling)
#
# #396 P2: complement to build-deb.sh/build-rpm.sh. Those two shell dpkg-deb /
# rpmbuild directly and only run correctly on a matching native Linux host.
# This script drives nfpm (pure-Go, 0-dependency packager) against the same
# staged layout (systemd unit, completions, license, user-creation scripts) —
# buildable from macOS, Linux, or anywhere `brew install nfpm` / a Go binary
# reaches. It does NOT replace build-deb.sh/build-rpm.sh; both paths stay
# available (native tooling produces byte-identical semantics when you have
# it; nfpm unblocks packaging when you don't).
#
# Requires: nfpm (https://nfpm.goreleaser.com) — brew install nfpm
#
# Usage:
#   ./scripts/packaging/build-nfpm.sh deb                    # uses target/release/zeus
#   ./scripts/packaging/build-nfpm.sh rpm /path/to/zeus       # custom binary path
#   ZEUS_VERSION=1.0.0 ./scripts/packaging/build-nfpm.sh deb  # override version
#
# Output: dist/zeus_<version>_<arch>.deb  or  dist/zeus-<version>-1.<arch>.rpm
# ═══════════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# ── Args ──────────────────────────────────────────────────────────────────────
PACKAGER="${1:-}"
if [[ "$PACKAGER" != "deb" && "$PACKAGER" != "rpm" ]]; then
    echo "Usage: $0 <deb|rpm> [binary-path]"
    exit 1
fi
BINARY="${2:-$REPO_ROOT/target/release/zeus}"

# ── Validate nfpm is available ───────────────────────────────────────────────
if ! command -v nfpm >/dev/null 2>&1; then
    echo "Error: nfpm not found. Install it:"
    echo "  macOS:  brew install nfpm"
    echo "  Linux:  see https://nfpm.goreleaser.com/install/"
    exit 1
fi

# ── Validate binary exists ───────────────────────────────────────────────────
if [ ! -f "$BINARY" ]; then
    echo "Error: zeus binary not found at $BINARY"
    echo "Run 'cargo build --release --bin zeus' first."
    exit 1
fi

# ── Configuration ────────────────────────────────────────────────────────────
VERSION="${ZEUS_VERSION:-0.1.0}"

# nfpm arch nomenclature differs slightly per-packager (rpm wants x86_64/aarch64,
# deb wants amd64/arm64) — nfpm's deb packager auto-maps go-arch names, but rpm
# does not, so pass the go-arch form and let nfpm's --packager handle mapping.
#
# IMPORTANT: this must be the arch of $BINARY, not necessarily this build
# host's arch — nfpm's whole point is packaging a cross-compiled target
# binary (e.g. linux-amd64) from a non-matching host (e.g. macOS/arm64).
# Callers that know the target arch should set ZEUS_NFPM_ARCH explicitly;
# the uname -m fallback below is only correct for same-host native packaging.
if [[ -n "${ZEUS_NFPM_ARCH:-}" ]]; then
    NFPM_ARCH="$ZEUS_NFPM_ARCH"
else
    MACHINE="$(uname -m)"
    case "$MACHINE" in
        x86_64)  NFPM_ARCH="amd64" ;;
        aarch64|arm64) NFPM_ARCH="arm64" ;;
        armv7l)  NFPM_ARCH="arm" ;;
        *)       NFPM_ARCH="$MACHINE" ;;
    esac
fi

DIST_DIR="$REPO_ROOT/dist"
STAGE="$DIST_DIR/nfpm-stage-$$"
mkdir -p "$STAGE/completions" "$STAGE/scripts"

echo "Building zeus ${VERSION} .${PACKAGER} for ${NFPM_ARCH} via nfpm..."

# ── Generate shell completions (best-effort, matches build-deb.sh behavior) ──
"$BINARY" completion bash > "$STAGE/completions/zeus.bash" 2>/dev/null \
    && echo "  Generated bash completions" \
    || { echo "  Warning: could not generate bash completions"; : > "$STAGE/completions/zeus.bash"; }

"$BINARY" completion zsh > "$STAGE/completions/_zeus" 2>/dev/null \
    && echo "  Generated zsh completions" \
    || { echo "  Warning: could not generate zsh completions"; : > "$STAGE/completions/_zeus"; }

"$BINARY" completion fish > "$STAGE/completions/zeus.fish" 2>/dev/null \
    && echo "  Generated fish completions" \
    || { echo "  Warning: could not generate fish completions"; : > "$STAGE/completions/zeus.fish"; }

# ── License ───────────────────────────────────────────────────────────────────
LICENSE_FILE="$STAGE/copyright"
if [ -f "$REPO_ROOT/LICENSE" ]; then
    cp "$REPO_ROOT/LICENSE" "$LICENSE_FILE"
elif [ -f "$REPO_ROOT/LICENSE-MIT" ]; then
    cp "$REPO_ROOT/LICENSE-MIT" "$LICENSE_FILE"
else
    cat > "$LICENSE_FILE" << 'LICEOF'
Zeus is dual-licensed under MIT OR Apache-2.0.
See https://github.com/zeuslabai/Zeus for full license text.
LICEOF
fi

# ── Pre/post/pre-remove scripts (same behavior as build-deb.sh/zeus.spec) ────
cat > "$STAGE/scripts/preinstall.sh" << 'EOF'
#!/bin/sh
set -e
# Create zeus system user if it doesn't exist (for gateway service)
if ! getent group zeus >/dev/null 2>&1; then
    groupadd --system zeus 2>/dev/null || true
fi
if ! getent passwd zeus >/dev/null 2>&1; then
    useradd --system --gid zeus --create-home --home-dir /home/zeus zeus 2>/dev/null || true
fi
EOF

cat > "$STAGE/scripts/postinstall.sh" << 'EOF'
#!/bin/sh
set -e
mkdir -p /home/zeus/.zeus
chown -R zeus:zeus /home/zeus/.zeus 2>/dev/null || true
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload 2>/dev/null || true
fi
echo ""
echo "Zeus installed successfully!"
echo ""
echo "  Quick start:    zeus"
echo "  Setup wizard:   zeus setup"
echo "  API gateway:    zeus gateway"
echo "  Start service:  systemctl enable --now zeus-gateway"
echo ""
EOF

cat > "$STAGE/scripts/preremove.sh" << 'EOF'
#!/bin/sh
set -e
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop zeus-gateway 2>/dev/null || true
    systemctl disable zeus-gateway 2>/dev/null || true
fi
EOF

chmod 755 "$STAGE/scripts/"*.sh

# ── Render nfpm.yaml ──────────────────────────────────────────────────────────
# nfpm's own env-var expansion (os.ExpandEnv) covers scalar fields (version,
# arch, description) but NOT contents[].src — that field is consumed as a raw
# glob pattern before expansion runs, so ${NFPM_BINARY} passes through literally
# and the glob match fails. Rather than pull in `envsubst`/gettext as an extra
# host dependency (which would undercut nfpm's own zero-dependency pitch), do
# the substitution ourselves with sed — we control the template and know
# exactly which variables it references.
RENDERED_YAML="$STAGE/nfpm.rendered.yaml"
sed \
    -e "s|\${NFPM_ARCH}|${NFPM_ARCH}|g" \
    -e "s|\${NFPM_VERSION}|${VERSION}|g" \
    -e "s|\${NFPM_BINARY}|${BINARY}|g" \
    -e "s|\${NFPM_REPO_ROOT}|${REPO_ROOT}|g" \
    -e "s|\${NFPM_COMPLETIONS_DIR}|${STAGE}/completions|g" \
    -e "s|\${NFPM_LICENSE_FILE}|${LICENSE_FILE}|g" \
    -e "s|\${NFPM_SCRIPTS_DIR}|${STAGE}/scripts|g" \
    "$SCRIPT_DIR/nfpm.yaml" > "$RENDERED_YAML"

# ── Build via nfpm ────────────────────────────────────────────────────────────
mkdir -p "$DIST_DIR"
nfpm package -f "$RENDERED_YAML" -p "$PACKAGER" -t "$DIST_DIR/"

echo ""
echo "Built via nfpm: see $DIST_DIR/ for the .${PACKAGER} artifact"

# Clean up staging
rm -rf "$STAGE"
