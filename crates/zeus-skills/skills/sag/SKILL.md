# sag

Signal CLI wrapper for encrypted messaging.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Signal messaging assistant. Help users send and receive encrypted messages via Signal using the signal-cli command-line interface.

## Tools

### signal_send
Send a Signal message.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string",
      "description": "Recipient phone number with country code"
    },
    "message": {
      "type": "string"
    }
  },
  "required": ["to", "message"]
}
```

### signal_send_group
Send message to a Signal group.
```json
{
  "type": "object",
  "properties": {
    "group_id": {
      "type": "string"
    },
    "message": {
      "type": "string"
    }
  },
  "required": ["group_id", "message"]
}
```

### signal_receive
Receive pending messages.
```json
{
  "type": "object",
  "properties": {
    "timeout": {
      "type": "integer",
      "default": 5,
      "description": "Timeout in seconds"
    }
  }
}
```

### signal_list_groups
List Signal groups.
```json
{
  "type": "object",
  "properties": {}
}
```

### signal_list_contacts
List Signal contacts.
```json
{
  "type": "object",
  "properties": {}
}
```

### signal_send_attachment
Send a file via Signal.
```json
{
  "type": "object",
  "properties": {
    "to": {
      "type": "string"
    },
    "file": {
      "type": "string",
      "description": "Path to file"
    },
    "message": {
      "type": "string"
    }
  },
  "required": ["to", "file"]
}
```

### signal_verify
Verify safety numbers with a contact.
```json
{
  "type": "object",
  "properties": {
    "contact": {
      "type": "string"
    }
  },
  "required": ["contact"]
}
```

## Commands

### send
```bash
signal-cli -u $SIGNAL_PHONE send -m "{message}" "{to}"
```

### send_group
```bash
signal-cli -u $SIGNAL_PHONE send -g "{group_id}" -m "{message}"
```

### receive
```bash
signal-cli -u $SIGNAL_PHONE receive --timeout {timeout}
```

### list_groups
```bash
signal-cli -u $SIGNAL_PHONE listGroups -d
```

### list_contacts
```bash
signal-cli -u $SIGNAL_PHONE listContacts
```

### send_attachment
```bash
signal-cli -u $SIGNAL_PHONE send -m "{message}" -a "{file}" "{to}"
```

## Environment
- SIGNAL_PHONE

## Permissions
- shell
- network
