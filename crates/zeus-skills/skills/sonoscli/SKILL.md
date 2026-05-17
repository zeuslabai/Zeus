# sonoscli

Control Sonos speakers via the SoCo library or sonos CLI.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Sonos multi-room audio assistant. Help users control playback, manage groups, adjust volume, and play music across their Sonos speaker system.

## Tools

### sonos_list
List all Sonos speakers.
```json
{
  "type": "object",
  "properties": {}
}
```

### sonos_play
Start or resume playback.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string",
      "description": "Speaker name"
    },
    "uri": {
      "type": "string",
      "description": "Music URI to play"
    }
  },
  "required": ["speaker"]
}
```

### sonos_pause
Pause playback.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    }
  },
  "required": ["speaker"]
}
```

### sonos_volume
Set speaker volume.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    },
    "level": {
      "type": "integer",
      "minimum": 0,
      "maximum": 100
    }
  },
  "required": ["speaker", "level"]
}
```

### sonos_next
Skip to next track.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    }
  },
  "required": ["speaker"]
}
```

### sonos_previous
Go to previous track.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    }
  },
  "required": ["speaker"]
}
```

### sonos_group
Group speakers together.
```json
{
  "type": "object",
  "properties": {
    "master": {
      "type": "string",
      "description": "Master speaker name"
    },
    "members": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Speakers to join"
    }
  },
  "required": ["master", "members"]
}
```

### sonos_ungroup
Ungroup a speaker.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    }
  },
  "required": ["speaker"]
}
```

### sonos_now_playing
Get current track info.
```json
{
  "type": "object",
  "properties": {
    "speaker": {
      "type": "string"
    }
  },
  "required": ["speaker"]
}
```

## Commands

### list
```bash
sonos list
```

### play
```bash
sonos "{speaker}" play
```

### pause
```bash
sonos "{speaker}" pause
```

### volume
```bash
sonos "{speaker}" volume {level}
```

### next
```bash
sonos "{speaker}" next
```

### previous
```bash
sonos "{speaker}" previous
```

### group
```bash
sonos "{master}" group "{members}"
```

### now_playing
```bash
sonos "{speaker}" track
```

## Permissions
- shell
- network
