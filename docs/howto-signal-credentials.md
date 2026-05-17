# How to Set Up Signal CLI for Zeus

Zeus connects to Signal via `signal-cli`, a command-line interface that links as a secondary device on your account. Your Signal app on your phone keeps working normally.

## Step 1: Install signal-cli

**macOS:**
```bash
brew install signal-cli
```

**Linux (Debian/Ubuntu):**
```bash
apt install signal-cli
```

**Linux (manual):**
Download the latest release from https://github.com/AsamK/signal-cli/releases and put it somewhere on your PATH (e.g. `/usr/local/bin/signal-cli`).

Verify it's installed:
```bash
signal-cli --version
```

## Step 2: Link as a Secondary Device

> ⚠️ Use `link`, not `register`. Registering will deactivate Signal on your phone.

```bash
signal-cli link --name "Zeus"
```

This prints a `sgnl://` URL. You need to scan it as a QR code:

```bash
# Generate a scannable QR code in the terminal (requires qrencode)
signal-cli link --name "Zeus" | qrencode -t ansiutf8

# Or on macOS, open it directly
signal-cli link --name "Zeus" | xargs -I{} open "{}"
```

Then on your phone:
1. Open Signal → **Settings** (top left)
2. Tap **Linked Devices**
3. Tap the **+** button
4. Scan the QR code

Once linked, `signal-cli` will receive and send messages using your phone number.

## Step 3: Find Your Phone Number

Your Signal phone number is the one registered on your phone — full E.164 format:
- ✅ `+15551234567` (US number)
- ✅ `+447911123456` (UK number)
- ❌ `5551234567` (missing country code — won't work)

## Step 4: Find the signal-cli Path

```bash
which signal-cli
# e.g. /usr/local/bin/signal-cli  or  /opt/homebrew/bin/signal-cli
```

## Step 5: Enter Credentials in Zeus

Run `zeus setup` or open the TUI and navigate to **Channels → Signal**:

- **signal-cli path** — paste the full path (e.g. `/opt/homebrew/bin/signal-cli`)
- **Phone number** — your number in E.164 format (e.g. `+15551234567`)

## Troubleshooting

- **"Not linked"** — run the `link` command again and scan the QR code
- **"No such file"** — double-check the path with `which signal-cli`
- **Messages not arriving** — make sure signal-cli is running in daemon mode: `signal-cli -u +YOUR_NUMBER daemon`
- **Relink needed** — if you reinstall Signal on your phone, you'll need to relink
