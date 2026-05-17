# blogwatcher

Monitor RSS/Atom feeds and blogs for new content.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a blog and feed monitoring assistant. Help users subscribe to RSS/Atom feeds, track blog updates, and get notified of new content. Summarize articles and filter by keywords.

## Tools

### feed_list
List subscribed feeds.
```json
{
  "type": "object",
  "properties": {}
}
```

### feed_add
Subscribe to a new feed.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "Feed URL (RSS/Atom)"
    },
    "name": {
      "type": "string",
      "description": "Display name"
    },
    "tags": {
      "type": "array",
      "items": {"type": "string"}
    }
  },
  "required": ["url"]
}
```

### feed_remove
Unsubscribe from a feed.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string"
    }
  },
  "required": ["url"]
}
```

### feed_fetch
Fetch latest items from a feed.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "default": 10
    }
  },
  "required": ["url"]
}
```

### feed_check_all
Check all feeds for new items.
```json
{
  "type": "object",
  "properties": {
    "since": {
      "type": "string",
      "description": "Check items since date (ISO 8601)"
    }
  }
}
```

### feed_search
Search feed items by keyword.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "feed_url": {
      "type": "string",
      "description": "Specific feed (optional)"
    }
  },
  "required": ["query"]
}
```

### blog_discover
Discover RSS feed URL from a website.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "Website URL"
    }
  },
  "required": ["url"]
}
```

## Commands

### fetch_feed
```bash
curl -sL "{url}" | xq -r '.rss.channel.item[:10] | .[] | "\(.title) - \(.link)"' 2>/dev/null || \
curl -sL "{url}" | xq -r '.feed.entry[:10] | .[] | "\(.title) - \(.link["@href"] // .link)"' 2>/dev/null
```

### discover_feed
```bash
curl -sL "{url}" | grep -oP '(?<=href=")[^"]*(?:rss|feed|atom)[^"]*' | head -5
```

## Permissions
- shell
- network
- filesystem
