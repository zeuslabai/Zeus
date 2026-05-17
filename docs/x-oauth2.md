# X (Twitter) OAuth 2.0 PKCE — Wiring Guide

Zeus's X adapter supports **three** auth modes. In 2024+ X requires OAuth 2.0
user-context for most write operations (posting tweets, sending DMs), so
OAuth 2.0 is the recommended path.

**Priority order** (highest first — see `write_auth_header()` in
`crates/zeus-channels/src/x.rs`):

1. **OAuth 2.0 user-context bearer** — `oauth2_access_token`
2. **OAuth 1.0a user-context** — `api_key` + `api_secret` + `access_token` + `access_token_secret`
3. **App-Only bearer** — `bearer_token` (read-only on v2)

The adapter picks the highest-priority credential present at request time.

---

## Quick path: paste a token into config.toml

If you already have an OAuth 2.0 access token (e.g. from the X developer
portal's Playground or your own PKCE flow), drop it straight in:

```toml
[channels.x]
client_id           = "your-x-client-id"
oauth2_access_token = "your-oauth2-access-token"
# optional — enables proactive refresh
oauth2_refresh_token = "your-refresh-token"
oauth2_expires_at    = 1735689600   # unix epoch seconds
```

Restart the gateway. The adapter will pick up the OAuth 2.0 token on its
next poll and use it for all requests.

## Runtime token updates (no restart)

Callers holding an `XAdapter` can rotate the token live:

```rust
adapter.set_oauth2_token(
    new_access_token,
    Some(new_refresh_token),
    Some(new_expires_at),
).await;
```

The polling loop re-reads the live token every cycle, so a refresh takes
effect on the next mention fetch. Use `current_oauth2_token()` to snapshot
the active credential for persistence back to `config.toml`.

## TUI onboarding — current status

The ChanConfig step of the wizard collects the seven classic X fields
(`bearer_token`, `api_key`, `api_secret`, `access_token`,
`access_token_secret`, `client_id`, `client_secret`).

A dedicated **"Login with X" PKCE step** is on the backlog — it would open
the authorize URL in a browser, capture the redirect, run the token
exchange, and write `oauth2_access_token` to config automatically. Until
that lands, follow the "Quick path" above.

## Backward compatibility

All three new fields are `#[serde(default)]`. Existing config.toml files
with only OAuth 1.0a or bearer creds continue to load and work unchanged.
