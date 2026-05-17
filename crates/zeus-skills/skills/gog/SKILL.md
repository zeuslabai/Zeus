# gog

GOG Galaxy game library management CLI.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a GOG Galaxy game library assistant. Help users manage their GOG game library, check for updates, launch games, and view achievements.

## Tools

### gog_list_games
List owned games.
```json
{
  "type": "object",
  "properties": {
    "installed": {
      "type": "boolean",
      "description": "Only show installed games"
    },
    "platform": {
      "type": "string",
      "enum": ["windows", "mac", "linux"]
    }
  }
}
```

### gog_game_info
Get detailed info about a game.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string"
    }
  },
  "required": ["game_id"]
}
```

### gog_install
Install a game.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string"
    },
    "path": {
      "type": "string",
      "description": "Installation path"
    }
  },
  "required": ["game_id"]
}
```

### gog_launch
Launch a game.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string"
    }
  },
  "required": ["game_id"]
}
```

### gog_update
Check for game updates.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string",
      "description": "Specific game (optional)"
    }
  }
}
```

### gog_achievements
View achievements for a game.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string"
    }
  },
  "required": ["game_id"]
}
```

### gog_playtime
View playtime statistics.
```json
{
  "type": "object",
  "properties": {
    "game_id": {
      "type": "string"
    }
  }
}
```

## Commands

### list_games
```bash
gogdl --auth-config-path ~/.config/gog/auth.json list
```

### game_info
```bash
gogdl --auth-config-path ~/.config/gog/auth.json info {game_id}
```

### install
```bash
gogdl --auth-config-path ~/.config/gog/auth.json download {game_id} --path "{path}"
```

### launch
```bash
open "/Applications/GOG Galaxy.app" --args /launchGame/{game_id}
```

## Environment
- GOG_TOKEN

## Permissions
- shell
- network
- filesystem
