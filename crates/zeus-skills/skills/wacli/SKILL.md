# wacli

WhatsApp CLI interface for messaging via WhatsApp Web bridge.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a WhatsApp messaging assistant. Help users send and receive WhatsApp messages, manage contacts, and handle media via the WhatsApp Web bridge.

## Tools

### wa_send
Send a WhatsApp message.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string",
      "description": "Phone number with country code (e.g., +1234567890)"
    },
    "message": {
      "type": "string"
    }
  },
  "required": ["to", "message"]
}
```

### wa_send_media
Send media file via WhatsApp.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string"
    },
    "file": {
      "type": "string",
      "description": "Path to media file"
    },
    "caption": {
      "type": "string"
    }
  },
  "required": ["to", "file"]
}
```

### wa_list_chats
List recent chats.
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

### wa_read_chat
Read messages from a chat.
```json
{
  "type": "object",
  "properties": {
    "chat_id": {
      "type": "string",
      "description": "Phone number or group ID"
    },
    "limit": {
      "type": "integer",
      "default": 50
    }
  },
  "required": ["chat_id"]
}
```

### wa_status
Check WhatsApp connection status.
```json
{
  "type": "object",
  "properties": {}
}
```

### wa_contacts
List contacts.
```json
{
  "type": "object",
  "properties": {
    "search": {
      "type": "string",
      "description": "Search filter"
    }
  }
}
```

### wa_groups
List groups.
```json
{
  "type": "object",
  "properties": {}
}
```

## Commands

### send
```bash
wacli send "{to}" "{message}"
```

### send_media
```bash
wacli send-media "{to}" "{file}" --caption "{caption}"
```

### list_chats
```bash
wacli chats --limit {limit}
```

### read_chat
```bash
wacli messages "{chat_id}" --limit {limit}
```

### status
```bash
wacli status
```

### contacts
```bash
wacli contacts
```

### groups
```bash
wacli groups
```

## Permissions
- shell
- network
