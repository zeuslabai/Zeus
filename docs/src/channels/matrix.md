# Matrix

Zeus connects to Matrix using the **matrix-sdk** v0.16 crate, a native Rust implementation of the Matrix protocol. This provides direct integration without external dependencies or bridge processes.

## Configuration

Matrix credentials are set via environment variables rather than `config.toml`:

| Variable | Description |
|----------|-------------|
| `MATRIX_HOMESERVER` | Matrix homeserver URL (e.g., `https://matrix.org`) |
| `MATRIX_USER` | Matrix username (e.g., `@zeus:matrix.org`) |
| `MATRIX_PASSWORD` | Matrix account password |

Set these in your shell profile:

```bash
export MATRIX_HOMESERVER="https://matrix.org"
export MATRIX_USER="@zeus:matrix.org"
export MATRIX_PASSWORD="your_password"
```

## Setup

### 1. Create a Matrix Account

Create an account on any Matrix homeserver. Popular options:

- [matrix.org](https://app.element.io/#/register) -- The largest public homeserver.
- Self-hosted -- Run your own homeserver with [Synapse](https://github.com/element-hq/synapse) or [Conduit](https://conduit.rs/).

It is recommended to create a dedicated account for Zeus rather than using your personal account.

### 2. Set Environment Variables

Export the three required environment variables as shown above.

### 3. Verify Connection

Start Zeus with the gateway to test the Matrix connection:

```bash
zeus gateway
```

Check the logs for successful Matrix login confirmation.

## Authentication

Zeus uses password login for the initial connection. After successful authentication, the session token is persisted and restored on subsequent connections, avoiding repeated password authentication. If the token expires or becomes invalid, Zeus falls back to password login automatically.

## Features

- Send and receive messages in Matrix rooms
- Native Rust implementation via matrix-sdk (no external bridge)
- Automatic session token persistence and restore
- End-to-end encryption support via the matrix-sdk crypto module
- Supports any Matrix homeserver (matrix.org, self-hosted, etc.)

## Limitations

- **Password login only** -- SSO and other login methods are not currently supported.
- **Room invites** -- Zeus must be invited to rooms before it can participate. Auto-join on invite can be configured.
- **Media** -- Text messages are the primary focus. Media message handling is limited.
