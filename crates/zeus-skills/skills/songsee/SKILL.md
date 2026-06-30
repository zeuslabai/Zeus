# songsee

Identify songs using audio fingerprinting (Shazam API or audd.io).

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a music identification assistant. Help users identify songs from audio files, microphone recordings, or streaming content using audio fingerprinting services.

## Tools

### song_identify_file
Identify a song from an audio file.
```json
{
  "type": "object",
  "properties": {
    "file": {
      "type": "string",
      "description": "Path to audio file"
    }
  },
  "required": ["file"]
}
```

### song_identify_mic
Record from microphone and identify.
```json
{
  "type": "object",
  "properties": {
    "duration": {
      "type": "integer",
      "default": 10,
      "description": "Recording duration in seconds"
    }
  }
}
```

### song_identify_url
Identify song from a URL (YouTube, etc.).
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "URL containing audio"
    }
  },
  "required": ["url"]
}
```

### song_lyrics
Get lyrics for a song.
```json
{
  "type": "object",
  "properties": {
    "artist": {
      "type": "string"
    },
    "title": {
      "type": "string"
    }
  },
  "required": ["artist", "title"]
}
```

### song_search
Search for song metadata.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query (lyrics, title, artist)"
    }
  },
  "required": ["query"]
}
```

## Commands

### identify_file
```bash
curl -s -X POST "https://api.audd.io/" \
  -F "file=@{file}" \
  -F "api_token=$AUDD_API_TOKEN" \
  -F "return=lyrics,spotify"
```

### identify_mic
```bash
rec -q -r 44100 -c 1 /tmp/song_sample.wav trim 0 {duration} && \
curl -s -X POST "https://api.audd.io/" \
  -F "file=@/tmp/song_sample.wav" \
  -F "api_token=$AUDD_API_TOKEN"
```

### identify_url
```bash
curl -s -X POST "https://api.audd.io/" \
  -F "url={url}" \
  -F "api_token=$AUDD_API_TOKEN"
```

### lyrics
```bash
curl -s "https://api.audd.io/findLyrics/?q={artist}%20{title}&api_token=$AUDD_API_TOKEN"
```

## Environment
- AUDD_API_TOKEN

## Permissions
- shell
- network
- microphone
