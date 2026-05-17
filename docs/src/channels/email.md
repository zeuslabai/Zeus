# Email

Zeus connects to email using **lettre** for sending (SMTP) and **async-imap** for receiving (IMAP IDLE). This allows Zeus to monitor an inbox and respond to incoming emails automatically.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.email]
smtp_host = "smtp.gmail.com"
imap_host = "imap.gmail.com"
username = "you@gmail.com"
password = "app-password"
```

| Field | Description |
|-------|-------------|
| `smtp_host` | SMTP server hostname for sending email |
| `imap_host` | IMAP server hostname for receiving email |
| `username` | Email address used for authentication |
| `password` | Password or app-specific password |

## Setup

### Gmail

Gmail requires an app-specific password when two-factor authentication is enabled (which is recommended).

1. Go to your [Google Account Security settings](https://myaccount.google.com/security).
2. Under **Signing in to Google**, select **App passwords** (requires 2FA to be enabled).
3. Generate a new app password for "Mail" on "Other (Custom name)".
4. Use the generated 16-character password as your `password` in the config.

Gmail SMTP and IMAP settings:

| Setting | Value |
|---------|-------|
| SMTP Host | `smtp.gmail.com` |
| SMTP Port | `587` (STARTTLS) or `465` (SSL) |
| IMAP Host | `imap.gmail.com` |
| IMAP Port | `993` (SSL) |

### Other Providers

Use the SMTP and IMAP hostnames provided by your email service. The `password` field should contain whatever credential your provider requires (standard password, app password, or OAuth token depending on the service).

## Features

- Send email via SMTP (TLS encrypted)
- Receive email via IMAP IDLE (real-time push notification of new messages)
- Automatic monitoring of the inbox when the gateway daemon is running
- Message chunking for long responses

## Limitations

- Only plaintext email content is processed. HTML emails have their text extracted.
- Attachments are not currently processed.
- Some email providers may require enabling "less secure app access" or generating app-specific passwords.
