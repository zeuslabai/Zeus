# Telegram

Zeus connects to Telegram using the **grammers-client** library, which implements the MTProto protocol directly. This provides a full Telegram client connection rather than the limited Bot API, enabling access to user accounts, groups, and channels.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.telegram]
api_id = 12345
api_hash = "your_api_hash"
phone = "+1234567890"
```

| Field | Description |
|-------|-------------|
| `api_id` | Telegram API application ID (integer) |
| `api_hash` | Telegram API application hash (string) |
| `phone` | Phone number associated with the Telegram account |

## Setup

### 1. Obtain API Credentials

1. Go to [https://my.telegram.org](https://my.telegram.org) and log in with your phone number.
2. Navigate to **API development tools**.
3. Create a new application if you do not already have one.
4. Note the **App api_id** and **App api_hash**.

### 2. Configure Zeus

Add the credentials to your `config.toml` as shown above. The phone number should include the country code (e.g., `+1` for US).

### 3. First Run Authentication

On first connection, grammers-client will prompt for a verification code sent to your Telegram account. This is a one-time process -- the session is persisted for subsequent connections.

## Features

- Send and receive text messages to any chat (users, groups, channels)
- Full MTProto protocol support (not limited to Bot API)
- Session persistence for reconnection without re-authentication
- Message chunking for long responses

## Limitations

- The first connection requires interactive verification (code sent to Telegram).
- Two-factor authentication (2FA) may require the password during initial setup.
