# bear-notes

Interact with Bear notes app on macOS via URL schemes and AppleScript.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Bear notes assistant for macOS. Help users create, search, and organize notes in the Bear app. Use Bear's URL schemes and AppleScript for seamless integration.

## Tools

### bear_create
Create a new note in Bear.
```json
{
  "type": "object",
  "properties": {
    "title": {
      "type": "string",
      "description": "Note title"
    },
    "text": {
      "type": "string",
      "description": "Note content (markdown supported)"
    },
    "tags": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Tags to add"
    },
    "pin": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["title"]
}
```

### bear_search
Search notes in Bear.
```json
{
  "type": "object",
  "properties": {
    "term": {
      "type": "string",
      "description": "Search term"
    },
    "tag": {
      "type": "string",
      "description": "Filter by tag"
    }
  },
  "required": ["term"]
}
```

### bear_open
Open a specific note.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string",
      "description": "Note identifier"
    },
    "title": {
      "type": "string",
      "description": "Note title (alternative to id)"
    }
  }
}
```

### bear_add_text
Append or prepend text to an existing note.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string"
    },
    "text": {
      "type": "string"
    },
    "mode": {
      "type": "string",
      "enum": ["append", "prepend"],
      "default": "append"
    }
  },
  "required": ["id", "text"]
}
```

### bear_tags
List all tags.
```json
{
  "type": "object",
  "properties": {}
}
```

### bear_trash
Move a note to trash.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string"
    }
  },
  "required": ["id"]
}
```

## Commands

### create
```bash
open "bear://x-callback-url/create?title={title}&text={text}&tags={tags}"
```

### search
```bash
open "bear://x-callback-url/search?term={term}&tag={tag}"
```

### open_note
```bash
open "bear://x-callback-url/open-note?id={id}"
```

### add_text
```bash
open "bear://x-callback-url/add-text?id={id}&text={text}&mode={mode}"
```

## Permissions
- applescript
- url_scheme
