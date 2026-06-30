# imsg

iMessage CLI for macOS using AppleScript and Messages database.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are an iMessage assistant for macOS. Help users send messages, read conversations, and manage their Messages app via AppleScript integration.

## Tools

### imsg_send
Send an iMessage.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string",
      "description": "Phone number or email"
    },
    "message": {
      "type": "string"
    }
  },
  "required": ["to", "message"]
}
```

### imsg_read
Read recent messages from a conversation.
```json
{
  "type": "object",
  "properties": {
    "contact": {
      "type": "string",
      "description": "Phone number or email"
    },
    "limit": {
      "type": "integer",
      "default": 20
    }
  },
  "required": ["contact"]
}
```

### imsg_list_conversations
List recent conversations.
```json
{
  "type": "object",
  "properties": {
    "limit": {
      "type": "integer",
      "default": 20
    }
  }
}
```

### imsg_search
Search messages.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "limit": {
      "type": "integer",
      "default": 50
    }
  },
  "required": ["query"]
}
```

### imsg_unread
Get unread message count.
```json
{
  "type": "object",
  "properties": {}
}
```

### imsg_attachments
List attachments from conversations.
```json
{
  "type": "object",
  "properties": {
    "contact": {
      "type": "string"
    },
    "type": {
      "type": "string",
      "enum": ["image", "video", "audio", "all"],
      "default": "all"
    }
  }
}
```

## Commands

### send
```bash
osascript -e 'tell application "Messages" to send "{message}" to buddy "{to}"'
```

### list_conversations
```bash
sqlite3 ~/Library/Messages/chat.db "SELECT DISTINCT handle.id, chat.display_name FROM chat JOIN chat_handle_join ON chat.ROWID = chat_handle_join.chat_id JOIN handle ON chat_handle_join.handle_id = handle.ROWID ORDER BY chat.last_read_message_timestamp DESC LIMIT {limit};"
```

### read_messages
```bash
sqlite3 ~/Library/Messages/chat.db "SELECT datetime(message.date/1000000000 + 978307200, 'unixepoch', 'localtime') as date, message.is_from_me, message.text FROM message JOIN chat_message_join ON message.ROWID = chat_message_join.message_id JOIN chat ON chat_message_join.chat_id = chat.ROWID JOIN handle ON message.handle_id = handle.ROWID WHERE handle.id = '{contact}' ORDER BY message.date DESC LIMIT {limit};"
```

### search
```bash
sqlite3 ~/Library/Messages/chat.db "SELECT datetime(message.date/1000000000 + 978307200, 'unixepoch', 'localtime'), handle.id, message.text FROM message JOIN handle ON message.handle_id = handle.ROWID WHERE message.text LIKE '%{query}%' ORDER BY message.date DESC LIMIT {limit};"
```

## Permissions
- applescript
- filesystem
