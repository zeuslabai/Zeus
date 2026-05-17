#!/usr/bin/env bash
# sign-release.sh — Sign and notarize the zeus binary with an Apple Developer ID.
#
# Produces a hardened-runtime, timestamped, notarized binary ready for
# distribution outside the Mac App Store.
#
# Requirements:
#   - macOS host with Xcode Command Line Tools (codesign, xcrun notarytool, stapler)
#   - A "Developer ID Application" certificate installed in the login keychain
#     (issued to the Apple Developer account at mike@novaxai.ai)
#   - A notarization keychain profile pre-created once via:
#       xcrun notarytool store-credentials "zeus-notary" \
#           --apple-id mike@novaxai.ai \
#           --team-id "$ZEUS_TEAM_ID" \
#           --password "<app-specific-password>"
#
# Usage:
#   scripts/sign-release.sh [path/to/zeus-binary]
#
# Environment overrides:
#   ZEUS_SIGN_IDENTITY   Full codesign identity string
#                        (default: "Developer ID Application: Mike Hash (${ZEUS_TEAM_ID})")
#   ZEUS_TEAM_ID         Apple Developer Team ID (required if identity not set)
#   ZEUS_BUNDLE_ID       Binary identifier (default: com.zeus.agent)
#   ZEUS_NOTARY_PROFILE  notarytool keychain profile name (default: zeus-notary)
#   ZEUS_SKIP_NOTARIZE   Set to 1 to skip notarization (sign only)

set -euo pipefail

# --- Config ----------------------------------------------------------------

BINARY_PATH="${1:-target/release/zeus}"
BUNDLE_ID="${ZEUS_BUNDLE_ID:-com.zeus.agent}"
NOTARY_PROFILE="${ZEUS_NOTARY_PROFILE:-zeus-notary}"
TEAM_ID="${ZEUS_TEAM_ID:-}"

if [[ -n "${ZEUS_SIGN_IDENTITY:-}" ]]; then
    SIGN_IDENTITY="$ZEUS_SIGN_IDENTITY"
elif [[ -n "$TEAM_ID" ]]; then
    SIGN_IDENTITY="Developer ID Application: Mike Hash ($TEAM_ID)"
else
    SIGN_IDENTITY="Developer ID Application"
fi

# --- Helpers ---------------------------------------------------------------

log()  { printf '\033[1;34m[sign]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[sign]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[sign]\033[0m %s\n' "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || die "required tool not found: $1"
}

# --- Preflight -------------------------------------------------------------

[[ "$(uname -s)" == "Darwin" ]] || die "sign-release.sh must run on macOS"

require codesign
require xcrun
require ditto

[[ -f "$BINARY_PATH" ]] || die "binary not found: $BINARY_PATH"

log "binary       : $BINARY_PATH"
log "identifier   : $BUNDLE_ID"
log "sign identity: $SIGN_IDENTITY"

# --- Sign ------------------------------------------------------------------

log "codesigning with hardened runtime + secure timestamp…"
codesign \
    --sign "$SIGN_IDENTITY" \
    --identifier "$BUNDLE_ID" \
    --options runtime \
    --timestamp \
    --force \
    --verbose=2 \
    "$BINARY_PATH"

log "verifying signature…"
codesign --verify --deep --strict --verbose=2 "$BINARY_PATH"
codesign --display --verbose=4 "$BINARY_PATH" 2>&1 | sed 's/^/    /'

# --- Notarize --------------------------------------------------------------

if [[ "${ZEUS_SKIP_NOTARIZE:-0}" == "1" ]]; then
    warn "ZEUS_SKIP_NOTARIZE=1 — skipping notarization"
    log "done (signed, not notarized)"
    exit 0
fi

# notarytool requires a zip (or pkg/dmg). Build one next to the binary.
ZIP_PATH="${BINARY_PATH}.zip"
log "packaging for notarization: $ZIP_PATH"
rm -f "$ZIP_PATH"
ditto -c -k --keepParent "$BINARY_PATH" "$ZIP_PATH"

log "submitting to Apple notary service (profile: $NOTARY_PROFILE)…"
if ! xcrun notarytool submit "$ZIP_PATH" \
        --keychain-profile "$NOTARY_PROFILE" \
        --wait; then
    die "notarization failed — run: xcrun notarytool log <submission-id> --keychain-profile $NOTARY_PROFILE"
fi

# Stand-alone binaries can't be stapled (stapler only works on bundles, dmg, pkg).
# The notarization ticket is attached server-side and Gatekeeper fetches it online.
if xcrun stapler staple "$BINARY_PATH" 2>/dev/null; then
    log "stapled ticket to binary"
else
    log "stapler not applicable to raw binary (expected) — Gatekeeper will fetch ticket online"
fi

rm -f "$ZIP_PATH"

log "✅ signed + notarized: $BINARY_PATH"
