# gifgrep

Search and download GIFs from Giphy, Tenor, and other sources.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a GIF search assistant. Help users find the perfect GIF for any occasion by searching Giphy, Tenor, and other GIF libraries. Download and save GIFs locally when needed.

## Tools

### gif_search
Search for GIFs.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "limit": {
      "type": "integer",
      "default": 10
    },
    "rating": {
      "type": "string",
      "enum": ["g", "pg", "pg-13", "r"],
      "default": "g"
    },
    "source": {
      "type": "string",
      "enum": ["giphy", "tenor"],
      "default": "giphy"
    }
  },
  "required": ["query"]
}
```

### gif_trending
Get trending GIFs.
```json
{
  "type": "object",
  "properties": {
    "limit": {
      "type": "integer",
      "default": 10
    },
    "source": {
      "type": "string",
      "enum": ["giphy", "tenor"],
      "default": "giphy"
    }
  }
}
```

### gif_random
Get a random GIF by tag.
```json
{
  "type": "object",
  "properties": {
    "tag": {
      "type": "string",
      "description": "Tag to filter by"
    }
  }
}
```

### gif_download
Download a GIF to local file.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "GIF URL"
    },
    "output": {
      "type": "string",
      "description": "Output file path"
    }
  },
  "required": ["url", "output"]
}
```

### gif_to_video
Convert GIF to video (MP4).
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string",
      "description": "GIF file path"
    },
    "output": {
      "type": "string"
    }
  },
  "required": ["input", "output"]
}
```

### gif_optimize
Optimize/compress a GIF.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "lossy": {
      "type": "integer",
      "default": 80,
      "description": "Lossy compression level (0-200)"
    }
  },
  "required": ["input", "output"]
}
```

## Commands

### search_giphy
```bash
curl -s "https://api.giphy.com/v1/gifs/search?api_key=$GIPHY_API_KEY&q={query}&limit={limit}&rating={rating}" | jq '.data[] | {id, title, url: .images.original.url}'
```

### trending_giphy
```bash
curl -s "https://api.giphy.com/v1/gifs/trending?api_key=$GIPHY_API_KEY&limit={limit}" | jq '.data[] | {id, title, url: .images.original.url}'
```

### random_giphy
```bash
curl -s "https://api.giphy.com/v1/gifs/random?api_key=$GIPHY_API_KEY&tag={tag}" | jq '.data | {id, title, url: .images.original.url}'
```

### download
```bash
curl -sL "{url}" -o "{output}"
```

### to_video
```bash
ffmpeg -y -i "{input}" -movflags faststart -pix_fmt yuv420p -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" "{output}"
```

### optimize
```bash
gifsicle -O3 --lossy={lossy} "{input}" -o "{output}"
```

## Environment
- GIPHY_API_KEY

## Permissions
- shell
- network
- filesystem
