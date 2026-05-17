# Email Channel — Getting Credentials

Zeus uses SMTP (sending) and IMAP (receiving) to connect to your email account.
Ports are fixed: **587 (SMTP)** and **993 (IMAP)** with TLS.

## Gmail

Gmail requires an **App Password** — your regular password won't work if 2FA is enabled (and it should be).

1. Go to [myaccount.google.com](https://myaccount.google.com)
2. Security → 2-Step Verification → enable it if not already on
3. Security → **App passwords** (search for it if not visible)
4. Select app: "Mail", select device: "Other" → name it "Zeus"
5. Copy the 16-character password Google generates

**TUI fields:**
```
smtp_host:  smtp.gmail.com
imap_host:  imap.gmail.com
username:   yourname@gmail.com
password:   abcd efgh ijkl mnop   (the app password, spaces optional)
```

## Outlook / Hotmail

1. Sign in at [account.microsoft.com](https://account.microsoft.com)
2. Security → Advanced security options → App passwords → Create
3. Copy the generated password

```
smtp_host:  smtp-mail.outlook.com
imap_host:  outlook.office365.com
username:   yourname@outlook.com
password:   (app password)
```

## Generic Provider

Check your provider's help docs for "SMTP settings" and "IMAP settings".
Most providers list host names like `mail.example.com` or `smtp.example.com`.

> **Note:** Zeus hardcodes SMTP port 587 and IMAP port 993 with TLS.
> If your provider requires different ports, that's a known limitation — open an issue.
