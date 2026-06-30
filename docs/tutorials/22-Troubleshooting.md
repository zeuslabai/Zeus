# Troubleshooting

Common issues and solutions for Zeus.

## Installation & Build

### `cargo build` fails with `alsa-sys` error on FreeBSD

The `cpal` crate pulls ALSA on non-macOS platforms. Zeus gates this with `cfg(not(target_os = "freebsd"))`. If you see this error, ensure you're on the latest `main` branch.

### Binary is 282MB

You built in debug mode. Always use `cargo build --release` for deployments. Release binary is ~99MB.

### `Killed: 9` after installing binary on macOS

`sudo cp` invalidates the ad-hoc code signature. Fix:

```bash
sudo cp target/release/zeus /usr/local/bin/zeus
codesign --force --sign - /usr/local/bin/zeus
```

## Configuration

### Setup wizard runs every time

Check that `~/.zeus/config.toml` exists and contains `onboarding_complete = true`. If it keeps looping, the credential check may be failing — set at least one API key:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Config file not found

Zeus looks for config at `~/.zeus/config.toml`. Create the directory:

```bash
mkdir -p ~/.zeus
zeus onboard
```

## API Keys & Credentials

### `OPENAI_API_KEY not set`

Set it in `~/.zeus/.env`:

```bash
echo 'OPENAI_API_KEY=sk-...' >> ~/.zeus/.env
```

Then restart the gateway. The gateway reads `.env` on startup.

### `OnceLock` / env vars not picked up

Zeus reads environment variables at init time via `OnceLock`. After changing `.env`, you must restart the gateway/MCP server — changes won't take effect in a running process.

### Where do secrets go?

ALL secrets go in `~/.zeus/.env`. NEVER put them in `settings.json`, `config.toml`, or commit them to git.

```bash
# ~/.zeus/.env
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
DISCORD_BOT_TOKEN=...
TELEGRAM_BOT_TOKEN=...
```

## MCP Server

### MCP crashes or fails to connect

1. **Never change the binary path** — canonical path is `/usr/local/bin/zeus`
2. Check logs: `tail -f ~/.zeus/logs/mcp.log`
3. Verify the binary exists: `ls -la /usr/local/bin/zeus`
4. Rebuild if needed: `cargo build --release && sudo cp target/release/zeus /usr/local/bin/zeus && codesign --force --sign - /usr/local/bin/zeus`

### Claude Code disconnects after 5 minutes

The MCP idle timeout was increased to 365 days. Update to the latest build:

```bash
git pull && cargo build --release
sudo cp target/release/zeus /usr/local/bin/zeus
codesign --force --sign - /usr/local/bin/zeus
```

## Gateway

### Gateway panics on emoji in messages

Fixed in commit `9075cca` — byte-slicing into multi-byte emoji characters caused panics. Update to latest:

```bash
git pull && cargo build --release
```

The fix uses `is_char_boundary()` before string truncation.

### Gateway won't start — port in use

```bash
# Check what's using the port
lsof -i :3001

# Kill the old process
kill $(lsof -ti :3001)

# Or use a different port
zeus gateway --port 3002
```

### Stale missions stuck in Executing

The gateway runs `recover_stale_missions()` on boot. Missions stuck in Executing/Assembling for >5 minutes are marked Failed. Just restart the gateway:

```bash
# macOS
zeus daemon restart

# FreeBSD
sudo service zeus_gateway restart
```

## Discord Relay

### Discord bot not connecting (401)

Check your bot token:

```bash
curl -H "Authorization: Bot YOUR_TOKEN_HERE" https://discord.com/api/v10/users/@me
```

If you get 401, the token is stale. Regenerate it in Discord Developer Portal.

### Messages not relaying

Ensure `DISCORD_RELAY_CHANNEL_IDS` is set in `~/.zeus/.env`:

```bash
DISCORD_RELAY_CHANNEL_IDS=1475583517156180018
```

### Bot can't see other bots

Fixed in commit `f461470`. Update to latest build. The bot filter was preventing bot-to-bot communication.

## tmux Issues

### `No such file or directory` for tmux session

Zeus auto-detects tmux sessions now (commit `9075cca`). The relay runs `tmux list-sessions` to find the active session instead of hardcoding `zeus-0`.

If no tmux session exists:
```bash
tmux new-session -d -s zeus-0
```

### tmux path wrong

On Apple Silicon Macs with Homebrew: `/opt/homebrew/bin/tmux`
On Intel Macs or FreeBSD: `/usr/local/bin/tmux`

## Database

### SQLite "table already exists" but missing columns

`CREATE TABLE IF NOT EXISTS` doesn't add new columns to existing tables. Zeus uses `ALTER TABLE ADD COLUMN` migrations. If schema is out of date, delete and restart:

```bash
rm ~/.zeus/zeus.db
zeus gateway  # Recreates tables with latest schema
```

### Foreign key violations silently ignored

`PRAGMA foreign_keys` is per-connection and not persisted. Zeus sets it per-connection. If using `sqlite3` CLI directly:

```bash
sqlite3 ~/.zeus/zeus.db "PRAGMA foreign_keys=ON; ..."
```

## Memory System

### Memory search returns nothing

Ensure FTS5 index is built:

```bash
zeus memory rebuild-index
```

### Entity extraction fails

The memory system needs an LLM API key for entity extraction. Check that `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` is set.

## Performance

### OOM on Mac Mini

Common cause: debug build + gateway + Claude Code + browser = too much memory. Solutions:
1. Use release builds only
2. Close unnecessary browser tabs
3. Monitor with `Activity Monitor` or `top`

### Slow first response

First LLM call warms up the connection pool. Subsequent calls are faster. If consistently slow, check your provider's API status.

## FreeBSD Specific

### Deploy script location

The deploy script is at `deployment/deploy-freebsd.sh` (NOT `scripts/deploy-freebsd.sh`).

### Audio features unavailable

`cpal` (audio capture) doesn't support FreeBSD. Audio features are gated with `cfg(not(target_os = "freebsd"))`. STT/TTS still work via HTTP APIs (Whisper, Piper).

## Getting Help

- Run `zeus doctor` for automated diagnostics
- Check logs: `~/.zeus/logs/`
- Gateway logs: `/var/log/zeus-gateway.log` (FreeBSD) or `~/Library/Logs/zeus/` (macOS)
- Open an issue on GitHub

## What's Next

→ [[00-Welcome]] — Tutorial overview
→ [[16-Deployment]] — Production deployment
→ [[15-Security]] — Security configuration
