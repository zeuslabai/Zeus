#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════════
# Zeus — Windows MSI Builder (#396 P4)
#
# Drives wixl (msitools' WiX-like MSI builder) against zeus.wxs to produce a
# real Windows Installer .msi from macOS or Linux — no Windows box required.
# `brew install msitools` gets you wixl/wixl-heat/msiinfo/msiextract; verified
# directly (not from docs) that wixl only implements a SUBSET of real WiX
# syntax: e.g. `Directory` is not a valid CustomAction attribute in wixl
# 0.106 (throws a GObject property error) — use `FileKey` to resolve the exe
# path from a <File> element instead. See zeus.wxs for the annotated source.
#
# The installer's custom action calls the SAME command the #308 Windows
# daemon.rs path already implements natively:
#   zeus.exe daemon install   → schtasks /Create /TN ZeusGateway ...
#   zeus.exe daemon uninstall → schtasks /Delete /TN ZeusGateway ...
# The .wxs doesn't reimplement schtasks logic — it just invokes the binary's
# own install/uninstall path post-file-copy / pre-file-removal.
#
# Requires: msitools (wixl) — brew install msitools (macOS) /
#           apt install msitools (Debian/Ubuntu) / dnf install msitools (Fedora)
#
# Usage:
#   ./scripts/packaging/build-msi.sh                          # uses target/x86_64-pc-windows-gnu/release/zeus.exe
#   ./scripts/packaging/build-msi.sh /path/to/zeus.exe         # custom binary path
#   ZEUS_VERSION=1.0.0 ./scripts/packaging/build-msi.sh        # override version
# ═══════════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY="${1:-$REPO_ROOT/target/x86_64-pc-windows-gnu/release/zeus.exe}"

if [ ! -f "$BINARY" ]; then
    echo "Error: zeus.exe not found at $BINARY"
    echo "Cross-compile it first: cargo build --release --target x86_64-pc-windows-gnu"
    exit 1
fi

if ! command -v wixl >/dev/null 2>&1; then
    echo "Error: wixl not found. macOS: brew install msitools; Debian/Ubuntu: apt install msitools; Fedora: dnf install msitools"
    exit 1
fi

# ── Configuration ────────────────────────────────────────────────────────────
VERSION="${ZEUS_VERSION:-0.1.0}"

# MSI Version field requires strict N.N.N.N (4 numeric fields, no suffix) —
# unlike deb/rpm/nfpm which accept semver-with-suffix. Verified: wixl accepts
# non-conformant strings silently but real Windows Installer will reject them
# at install time, so normalize defensively rather than pass VERSION through
# raw. Strip any non-numeric/non-dot suffix, then pad to 4 fields.
msi_version() {
    local v="$1"
    # Keep only the leading run of digits and dots (drops -rc1, +build, etc.)
    v="$(echo "$v" | sed -E 's/^([0-9]+(\.[0-9]+)*).*/\1/')"
    local IFS=.
    read -ra parts <<< "$v"
    while [ "${#parts[@]}" -lt 4 ]; do
        parts+=("0")
    done
    echo "${parts[0]}.${parts[1]}.${parts[2]}.${parts[3]}"
}
MSI_VERSION="$(msi_version "$VERSION")"

# UpgradeCode must be STABLE across releases (it's what lets Windows Installer
# recognize "this MSI is an upgrade of that MSI" via the Upgrade table wired
# in zeus.wxs) — a fixed, checked-in GUID, not regenerated per build. Component
# GUIDs, by contrast, are also fixed here (one component each) since this MSI
# ships a single-file component tree with no per-build component churn.
UPGRADE_CODE="9c4f9b9a-9b7a-4e3c-9f2a-1a2b3c4d5e6f"
MAIN_COMPONENT_GUID="7e1d9f7a-3b1f-4e5a-8c9d-2f3a4b5c6d7e"
LICENSE_COMPONENT_GUID="3a5b7c9d-1e2f-4a6b-8c9d-0e1f2a3b4c5d"

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp "$BINARY" "$STAGE/zeus.exe"

if [ -f "$REPO_ROOT/LICENSE" ]; then
    cp "$REPO_ROOT/LICENSE" "$STAGE/LICENSE.txt"
elif [ -f "$REPO_ROOT/LICENSE-MIT" ]; then
    cp "$REPO_ROOT/LICENSE-MIT" "$STAGE/LICENSE.txt"
else
    echo "Warning: no LICENSE file found at repo root, shipping without one"
    : > "$STAGE/LICENSE.txt"
fi

# ── Render zeus.wxs ──────────────────────────────────────────────────────────
RENDERED_WXS="$STAGE/zeus.wxs"
sed \
    -e "s|__VERSION__|${MSI_VERSION}|g" \
    -e "s|__UPGRADE_CODE__|${UPGRADE_CODE}|g" \
    -e "s|__COMPONENT_GUID__|${MAIN_COMPONENT_GUID}|g" \
    -e "s|__LICENSE_GUID__|${LICENSE_COMPONENT_GUID}|g" \
    "$SCRIPT_DIR/zeus.wxs" > "$RENDERED_WXS"

# ── Build via wixl ────────────────────────────────────────────────────────────
DIST_DIR="$REPO_ROOT/dist"
mkdir -p "$DIST_DIR"
OUT_MSI="$DIST_DIR/zeus-${VERSION}-windows-amd64.msi"

# wixl resolves Source="zeus.exe"/"LICENSE.txt" relative to its own cwd, so
# run from $STAGE where both files + the rendered .wxs live together.
(cd "$STAGE" && wixl -v -o "$OUT_MSI" zeus.wxs)

echo ""
echo "Built: $OUT_MSI"
