# Signal

Zeus connects to Signal using **signal-cli**, a third-party command-line client for Signal. Zeus communicates with signal-cli via its JSON-RPC interface running as a subprocess.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.signal]
signal_cli_path = "/usr/local/bin/signal-cli"
phone_number = "+1234567890"
```

| Field | Description |
|-------|-------------|
| `signal_cli_path` | Path to the signal-cli binary |
| `phone_number` | Phone number registered with Signal |

## Setup

### 1. Install signal-cli

#### macOS (Homebrew)

```bash
brew install signal-cli
```

#### Linux

Download the latest release from [https://github.com/AsamK/signal-cli/releases](https://github.com/AsamK/signal-cli/releases) and extract it to a directory in your PATH.

signal-cli requires Java 17 or newer.

### 2. Register a Phone Number

Register a phone number with Signal via signal-cli:

```bash
signal-cli -u +1234567890 register
```

You will receive a verification code via SMS. Complete the verification:

```bash
signal-cli -u +1234567890 verify 123456
```

Alternatively, if you already use Signal on another device, you can link signal-cli as a secondary device:

```bash
signal-cli link -n "Zeus"
```

This displays a QR code URI that you scan with your Signal app under **Settings > Linked Devices**.

### 3. Test the Connection

Verify signal-cli is working:

```bash
signal-cli -u +1234567890 send -m "Hello from Zeus" +1987654321
```

### 4. Configure Zeus

Add the signal-cli path and phone number to your `config.toml` as shown above.

## Features

- Send and receive Signal messages
- End-to-end encrypted communication (Signal protocol)
- Runs signal-cli as a JSON-RPC subprocess managed by Zeus
- Supports both registered accounts and linked devices

## Limitations

- **signal-cli must be installed separately** -- Zeus does not bundle signal-cli.
- **Java dependency** -- signal-cli requires Java 17+.
- **One device per number** -- Registering a phone number with signal-cli will deactivate Signal on any phone using that number. Use the linked device method to avoid this.
- **No group management** -- Basic send/receive is supported; advanced group operations may be limited.
