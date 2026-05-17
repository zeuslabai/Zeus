# spotify

Control Spotify playback and search music via spotify-tui or AppleScript.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Spotify music assistant. Help users control playback, search for music, create playlists, and discover new tracks. Use the Spotify tools to interact with the Spotify application.

## Tools

### spotify_play
Play a track, album, artist, or playlist.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query or Spotify URI"
    },
    "type": {
      "type": "string",
      "enum": ["track", "album", "artist", "playlist"],
      "default": "track"
    }
  },
  "required": ["query"]
}
```

### spotify_pause
Pause playback.
```json
{
  "type": "object",
  "properties": {}
}
```

### spotify_next
Skip to next track.
```json
{
  "type": "object",
  "properties": {}
}
```

### spotify_previous
Go to previous track.
```json
{
  "type": "object",
  "properties": {}
}
```

### spotify_current
Get currently playing track info.
```json
{
  "type": "object",
  "properties": {}
}
```

### spotify_search
Search Spotify catalog.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "type": {
      "type": "string",
      "enum": ["track", "album", "artist", "playlist"],
      "default": "track"
    },
    "limit": {
      "type": "integer",
      "default": 10
    }
  },
  "required": ["query"]
}
```

### spotify_volume
Set playback volume.
```json
{
  "type": "object",
  "properties": {
    "level": {
      "type": "integer",
      "minimum": 0,
      "maximum": 100
    }
  },
  "required": ["level"]
}
```

## Commands

### play
```bash
osascript -e 'tell application "Spotify" to play track "spotify:track:{query}"'
```

### pause
```bash
osascript -e 'tell application "Spotify" to pause'
```

### next
```bash
osascript -e 'tell application "Spotify" to next track'
```

### previous
```bash
osascript -e 'tell application "Spotify" to previous track'
```

### current
```bash
osascript -e 'tell application "Spotify" to get {name of current track, artist of current track, album of current track}'
```

## Permissions
- applescript
- network
