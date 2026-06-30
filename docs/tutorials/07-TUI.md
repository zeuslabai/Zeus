# Terminal UI (TUI)

The TUI is Zeus's default interface — a full-screen terminal app built with Ratatui.

## Launch

```bash
zeus          # Default — opens TUI
zeus tui      # Explicit
```

## Screens

Navigate between screens with Tab/Shift-Tab or number keys:

| Key | Screen | Description |
|-----|--------|-------------|
| `1` | **Chat** | Main conversation interface |
| `2` | **Tools** | Browse and search all 212 tools |
| `3` | **Memory** | View workspace files and memory |
| `4` | **Agents** | Agent profiles and fleet status |
| `5` | **Status** | Model, provider, session info, subsystem health |
| `6` | **Help** | Keyboard shortcuts reference |
| `7` | **Settings** | Edit configuration in-place |
| `8` | **Teams** | Multi-agent teams |
| `9` | **Extensions** | Installed extensions and skills |
| `0` | **Sandbox** | Security sandbox policies |

## Chat Screen

The main screen. Type your message at the bottom and press **Enter** to send.

### Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | Insert newline (multi-line input) |
| `↑` / `↓` | Scroll through chat history |
| `Ctrl+C` | Exit TUI |
| `/clear` | Clear conversation / start new session |
| `/` | Search (in Tools screen) |

### What You'll See

- **Your messages** — Right-aligned, highlighted
- **Zeus responses** — Left-aligned, streaming token-by-token
- **Tool calls** — Shown inline with 🔧 icon, input args, and output
- **Errors** — Red-highlighted with details

### Multi-line Input

Press `Shift+Enter` to add newlines. Useful for pasting code or writing longer prompts. Press `Enter` alone to send.

## Tools Screen

Browse all 212 tools:
- Use `/` to search by name or description
- Tools grouped by category (Core, System, Files, Git, etc.)
- Select a tool to see its full schema and parameters

## Memory Screen

View and navigate workspace files:
- `AGENTS.md` — System prompt
- `SOUL.md` — Personality
- `USER.md` — User context
- `MEMORY.md` — Long-term facts
- Daily notes

## Status Screen

At-a-glance system health:
- Current model and provider
- Active session ID
- Workspace path
- Subsystem status (Mnemosyne, Nous, Aegis, etc.)

## Vim Mode

Enable vim-style navigation:

```toml
# ~/.zeus/config.toml
[tui]
vim_mode = true
```

With vim mode:
- `j`/`k` — scroll up/down in chat
- `i` — enter insert mode (start typing)
- `Esc` — back to normal mode
- `gg` — jump to top
- `G` — jump to bottom

## Theme

```toml
[tui]
theme = "dark"    # or "light"
```

## Tips

1. **Quick tool test**: Switch to Tools screen (press `2`), search for a tool, see its parameters
2. **Check memory**: Switch to Memory screen (press `3`) to see what Zeus knows about you
3. **Monitor health**: Status screen (press `5`) shows if all subsystems are connected
4. **Reset session**: Type `/clear` in chat to start fresh

## What's Next

→ [[08-API-Server]] — Use Zeus via REST API
→ [[04-Chat-and-Conversations]] — Chat features deep dive
