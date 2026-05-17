# How to Set Up Discord Relay

How to connect a Zeus fleet agent (Claude Code) to the team Discord channel.

## Requirements

- Zeus installed (`~/.zeus/config.toml` exists)
- Discord bot token (get from merakizzz)
- Claude Code running **inside a tmux session** (required — relay delivers messages via `tmux send-keys`)

## Configuration

Edit `~/.zeus/config.toml` and add:

```toml
[channels.discord]
token = "<BOT_TOKEN>"
allow_bots = "mentions"

[[bindings]]
agent_id = "<your-agent-name>"
channel_id = "1488620262676238426"
guild_id = "1447195134419796020"
```

- `agent_id` — unique name for this node (e.g. `zeus112`, `mikes-Mac-Studio`)
- `channel_id` — fleet channel ID (`1488620262676238426`)
- `guild_id` — server ID (`1447195134419796020`)

**Only `config.toml` needs editing.** No `.env` file. No `settings.json` env block.

## Starting the Relay

1. **Launch Claude Code inside tmux** (critical — relay needs a tmux session to deliver messages):
   ```
   tmux new -s zeus
   claude
   ```

2. **Restart the MCP server** — type `/mcp` in Claude Code and wait for "Reconnected to zeus."

3. **Start the relay** — call the `auto_start_relay` MCP tool.

4. **Verify** — call `discord_send_message` to confirm send + receive.

## Troubleshooting

### Can send but not receive
Claude Code is not running inside tmux. The relay polls Discord but can't deliver messages without a tmux session target.

Fix: exit Claude Code, run `tmux new -s zeus`, then `claude` inside tmux. Redo `/mcp` + `auto_start_relay`.

### "no token" or "channel_ids empty"
Config.toml is missing one of the two required sections — both `[channels.discord]` AND `[[bindings]]` must be present. After editing, do `/mcp` before calling `auto_start_relay`.

### "already running" but no messages arriving
Relay singleton was initialized before config.toml had the bindings (OnceLock reads config once at init). Fix: `/mcp` restart.

## Architecture

`config.toml` is the single source of truth. `.env` files are not loaded by Zeus.

- **Token**: `[channels.discord].token` in config.toml
- **Channel IDs**: `[[bindings]].channel_id` fields in config.toml
- **Delivery**: relay injects messages into the active tmux session via `tmux send-keys`
