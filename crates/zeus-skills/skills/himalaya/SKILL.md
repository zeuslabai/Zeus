# himalaya

CLI email client for IMAP/SMTP with offline support via himalaya.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are an email assistant using the himalaya CLI. Help users read, send, search, and manage emails directly from the terminal with efficient keyboard-driven workflows.

## Tools

### himalaya_list
List emails in a folder.
```json
{
  "type": "object",
  "properties": {
    "folder": {
      "type": "string",
      "default": "INBOX"
    },
    "page": {
      "type": "integer",
      "default": 1
    },
    "page_size": {
      "type": "integer",
      "default": 10
    },
    "account": {
      "type": "string",
      "description": "Account name from config"
    }
  }
}
```

### himalaya_read
Read an email by ID.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string",
      "description": "Email ID"
    },
    "folder": {
      "type": "string",
      "default": "INBOX"
    },
    "raw": {
      "type": "boolean",
      "default": false,
      "description": "Show raw email"
    }
  },
  "required": ["id"]
}
```

### himalaya_send
Send an email.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string",
      "description": "Recipient email"
    },
    "subject": {
      "type": "string"
    },
    "body": {
      "type": "string"
    },
    "cc": {
      "type": "string"
    },
    "bcc": {
      "type": "string"
    },
    "attachments": {
      "type": "array",
      "items": {"type": "string"},
      "description": "File paths to attach"
    }
  },
  "required": ["to", "subject", "body"]
}
```

### himalaya_reply
Reply to an email.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string"
    },
    "body": {
      "type": "string"
    },
    "all": {
      "type": "boolean",
      "default": false,
      "description": "Reply all"
    }
  },
  "required": ["id", "body"]
}
```

### himalaya_search
Search emails.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "IMAP search query"
    },
    "folder": {
      "type": "string",
      "default": "INBOX"
    }
  },
  "required": ["query"]
}
```

### himalaya_move
Move email to a folder.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string"
    },
    "to": {
      "type": "string",
      "description": "Destination folder"
    }
  },
  "required": ["id", "to"]
}
```

### himalaya_delete
Delete an email.
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

### list
```bash
himalaya list --folder "{folder}" --page {page} --page-size {page_size}
```

### read
```bash
himalaya read {id} --folder "{folder}"
```

### send
```bash
echo "{body}" | himalaya send --to "{to}" --subject "{subject}"
```

### search
```bash
himalaya search "{query}" --folder "{folder}"
```

### move
```bash
himalaya move {id} --to "{to}"
```

### delete
```bash
himalaya delete {id}
```

## Permissions
- shell
- network
