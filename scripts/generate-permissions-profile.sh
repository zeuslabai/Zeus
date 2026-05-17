#!/usr/bin/env sh
# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  Zeus — macOS Permissions Profile Generator                             ║
# ║  Generates a .mobileconfig profile granting Full Disk Access +          ║
# ║  Automation permissions to the Zeus agent bundle.                       ║
# ║                                                                          ║
# ║  Usage:  ./generate-permissions-profile.sh [output_path]                ║
# ║          (default output: /tmp/zeus-permissions.mobileconfig)           ║
# ║                                                                          ║
# ║  After generation, open the file and approve it in:                     ║
# ║    System Settings → Privacy & Security → Profiles                      ║
# ╚══════════════════════════════════════════════════════════════════════════╝
set -eu

BUNDLE_ID="${ZEUS_BUNDLE_ID:-com.zeus.agent}"
OUTPUT="${1:-/tmp/zeus-permissions.mobileconfig}"

# Generate stable-ish UUIDs (uuidgen is present on macOS by default)
if command -v uuidgen >/dev/null 2>&1; then
    PAYLOAD_UUID="$(uuidgen)"
    PROFILE_UUID="$(uuidgen)"
    TCC_UUID="$(uuidgen)"
else
    # Fallback: deterministic-ish fake UUIDs
    PAYLOAD_UUID="11111111-1111-1111-1111-111111111111"
    PROFILE_UUID="22222222-2222-2222-2222-222222222222"
    TCC_UUID="33333333-3333-3333-3333-333333333333"
fi

cat > "$OUTPUT" <<PROFILE_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>PayloadContent</key>
    <array>
        <dict>
            <key>PayloadDescription</key>
            <string>Grants Zeus agent Full Disk Access and Automation permissions.</string>
            <key>PayloadDisplayName</key>
            <string>Zeus Privacy Preferences</string>
            <key>PayloadIdentifier</key>
            <string>${BUNDLE_ID}.tcc.${TCC_UUID}</string>
            <key>PayloadOrganization</key>
            <string>Zeus Labs</string>
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
                        <string>identifier "${BUNDLE_ID}" and anchor apple generic</string>
                        <key>Comment</key>
                        <string>Full Disk Access for Zeus agent</string>
                        <key>Identifier</key>
                        <string>${BUNDLE_ID}</string>
                        <key>IdentifierType</key>
                        <string>bundleID</string>
                    </dict>
                </array>
                <key>AppleEvents</key>
                <array>
                    <dict>
                        <key>Allowed</key>
                        <true/>
                        <key>CodeRequirement</key>
                        <string>identifier "${BUNDLE_ID}" and anchor apple generic</string>
                        <key>Comment</key>
                        <string>Automation (AppleEvents) for Zeus agent</string>
                        <key>Identifier</key>
                        <string>${BUNDLE_ID}</string>
                        <key>IdentifierType</key>
                        <string>bundleID</string>
                        <key>AEReceiverIdentifier</key>
                        <string>com.apple.systemevents</string>
                        <key>AEReceiverIdentifierType</key>
                        <string>bundleID</string>
                        <key>AEReceiverCodeRequirement</key>
                        <string>identifier "com.apple.systemevents" and anchor apple</string>
                    </dict>
                </array>
                <key>Accessibility</key>
                <array>
                    <dict>
                        <key>Allowed</key>
                        <true/>
                        <key>CodeRequirement</key>
                        <string>identifier "${BUNDLE_ID}" and anchor apple generic</string>
                        <key>Comment</key>
                        <string>Accessibility control for Zeus agent</string>
                        <key>Identifier</key>
                        <string>${BUNDLE_ID}</string>
                        <key>IdentifierType</key>
                        <string>bundleID</string>
                    </dict>
                </array>
            </dict>
        </dict>
    </array>
    <key>PayloadDescription</key>
    <string>Grants the Zeus agent Full Disk Access and Automation permissions so it can operate autonomously on this Mac.</string>
    <key>PayloadDisplayName</key>
    <string>Zeus Agent Permissions</string>
    <key>PayloadIdentifier</key>
    <string>${BUNDLE_ID}.profile</string>
    <key>PayloadOrganization</key>
    <string>Zeus Labs</string>
    <key>PayloadRemovalDisallowed</key>
    <false/>
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
PROFILE_EOF

echo "✓ Profile generated: $OUTPUT"
echo "  Bundle ID: $BUNDLE_ID"
echo ""
echo "Next steps:"
echo "  1. Double-click the .mobileconfig (or run: open \"$OUTPUT\")"
echo "  2. Open System Settings → Privacy & Security → Profiles"
echo "  3. Review and install the 'Zeus Agent Permissions' profile"
echo "  4. Authenticate with admin password when prompted"
