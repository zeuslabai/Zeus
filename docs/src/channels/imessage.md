# iMessage

Zeus connects to iMessage via an **AppleScript bridge**, using the macOS Messages app directly. This channel requires no external credentials or API keys -- it uses the Messages app already configured on your Mac.

## Configuration

No configuration is needed in `config.toml`. The iMessage channel is available automatically on macOS when the Messages app is signed in to an iMessage account.

## Setup

### 1. Verify Messages App

Ensure the macOS Messages app is running and signed in to your Apple ID with iMessage enabled.

### 2. Grant Automation Permissions

When Zeus first sends an iMessage, macOS will prompt you to allow the application to control Messages. Grant this permission in **System Settings > Privacy & Security > Automation**.

You may also need to grant accessibility permissions in **System Settings > Privacy & Security > Accessibility** for AppleScript automation to function.

## Features

- Send iMessages to contacts and phone numbers
- Read recent messages from conversations
- List active conversations
- Uses the native Messages app (no third-party service)

## Limitations

- **macOS only** -- The AppleScript bridge requires macOS. This channel is not available on Linux or other platforms.
- **Messages app must be running** -- The Messages app needs to be open (it can be in the background).
- **Automation permissions required** -- macOS will prompt for permission to control Messages on first use.
- **No real-time push** -- Message receiving is poll-based via AppleScript, not event-driven. New messages are checked periodically.
- **iMessage only** -- SMS messages sent through iPhone relay are not guaranteed to work.
