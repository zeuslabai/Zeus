# Matrix Channel — Getting Credentials

Zeus connects to Matrix using the `matrix-sdk`. You can authenticate with either
a username/password or an access token.

## Option A: Username + Password

Straightforward if you have a Matrix account.

```
homeserver:    https://matrix.org          (or your own server)
username:      @yourname:matrix.org        (full Matrix ID)
password:      your-matrix-password
access_token:  (leave blank)
```

## Option B: Access Token (Recommended for bots)

More secure — Zeus never stores your password.

### Get your token from Element

1. Open [Element](https://app.element.io) in your browser
2. Log in to your account
3. Click your avatar → **Settings**
4. Scroll to **Help & About** → **Advanced**
5. Click **Access Token** → copy it

```
homeserver:    https://matrix.org
username:      @yourname:matrix.org
password:      (leave blank)
access_token:  syt_abc123...               (the token you copied)
```

> **Note:** Keep your access token secret — it grants full account access.
> Revoke it from Element settings if you ever need to rotate it.

## Self-Hosted Homeserver

If you run your own Synapse or Conduit instance:

```
homeserver:    https://matrix.yourdomain.com
username:      @yourname:yourdomain.com
```

Everything else is the same as above.

## Common Homeservers

| Provider | Homeserver URL |
|----------|---------------|
| matrix.org | `https://matrix.org` |
| Element Matrix Services | `https://ems.host` |
| Self-hosted | `https://matrix.yourdomain.com` |
