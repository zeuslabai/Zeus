#!/bin/bash
# generate-fda-profile.sh — Generate a macOS .mobileconfig profile
# that grants Full Disk Access + Automation to the Zeus binary.
#
# Usage: ./scripts/generate-fda-profile.sh [--install]
#   --install: automatically open the profile for user approval
#
# The profile grants:
#   - Full Disk Access (kTCCServiceSystemPolicyAllFiles)
#   - Accessibility (kTCCServiceAccessibility)
#
# Users approve once in System Settings; persists across rebuilds
# as long as the binary path + codesign identity stay the same.

set -euo pipefail

ZEUS_BINARY="${ZEUS_BINARY:-/usr/local/bin/zeus}"
BUNDLE_ID="${ZEUS_BUNDLE_ID:-com.zeus.agent}"
PROFILE_DIR="${HOME}/.zeus"
PROFILE_PATH="${PROFILE_DIR}/zeus-permissions.mobileconfig"
PROFILE_UUID=$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid 2>/dev/null || echo "A1B2C3D4-E5F6-7890-ABCD-EF1234567890")
PAYLOAD_UUID=$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid 2>/dev/null || echo "F0E1D2C3-B4A5-6789-0123-456789ABCDEF")

# Only run on macOS
if [ "$(uname)" != "Darwin" ]; then
    echo "Skipping FDA profile generation (not macOS)"
    exit 0
fi

# Get the code signing identity of the zeus binary
CODE_REQ=""
if [ -f "$ZEUS_BINARY" ]; then
    CODE_REQ=$(codesign -dr - "$ZEUS_BINARY" 2>&1 | grep "designated" | sed 's/designated => //' || true)
fi

# If no code requirement, use bundle ID as identifier
if [ -z "$CODE_REQ" ]; then
    CODE_REQ="identifier \"${BUNDLE_ID}\""
fi

mkdir -p "$PROFILE_DIR"

cat > "$PROFILE_PATH" << MOBILECONFIG
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>PayloadContent</key>
    <array>
        <dict>
            <key>PayloadDescription</key>
            <string>Privacy Preferences for Zeus AI Agent</string>
            <key>PayloadDisplayName</key>
            <string>Zeus Permissions</string>
            <key>PayloadIdentifier</key>
            <string>${BUNDLE_ID}.pppc</string>
            <key>PayloadType</key>
            <string>com.apple.TCC.configuration-profile-policy</string>
            <key>PayloadUUID</key>
            <string>${PAYLOAD_UUID}</string>
            <key>PayloadVersion</key>
            <integer>1</integer>
            <key>Services</key>
            <dict>
                <key>SystemPolicyAllFiles</key>
                <array>
                    <dict>
                        <key>Allowed</key>
                        <true/>
                        <key>CodeRequirement</key>
                        <string>${CODE_REQ}</string>
                        <key>Identifier</key>
                        <string>${BUNDLE_ID}</string>
                        <key>IdentifierType</key>
                        <string>bundleID</string>
                        <key>StaticCode</key>
                        <false/>
                    </dict>
                </array>
                <key>Accessibility</key>
                <array>
                    <dict>
                        <key>Allowed</key>
                        <true/>
                        <key>CodeRequirement</key>
                        <string>${CODE_REQ}</string>
                        <key>Identifier</key>
                        <string>${BUNDLE_ID}</string>
                        <key>IdentifierType</key>
                        <string>bundleID</string>
                        <key>StaticCode</key>
                        <false/>
                    </dict>
                </array>
            </dict>
        </dict>
    </array>
    <key>PayloadDescription</key>
    <string>Grants Zeus AI Agent Full Disk Access and Accessibility permissions.</string>
    <key>PayloadDisplayName</key>
    <string>Zeus AI Agent Permissions</string>
    <key>PayloadIdentifier</key>
    <string>${BUNDLE_ID}.profile</string>
    <key>PayloadOrganization</key>
    <string>Zeus</string>
    <key>PayloadScope</key>
    <string>System</string>
    <key>PayloadType</key>
    <string>Configuration</string>
    <key>PayloadUUID</key>
    <string>${PROFILE_UUID}</string>
    <key>PayloadVersion</key>
    <integer>1</integer>
</dict>
</plist>
MOBILECONFIG

echo "Generated FDA profile: ${PROFILE_PATH}"

if [ "${1:-}" = "--install" ]; then
    echo "Opening profile for installation..."
    echo "Approve in System Settings > Profiles when prompted."
    open "$PROFILE_PATH"
fi
