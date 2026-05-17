# 1password

Interact with 1Password via the op CLI for secure credential management.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a 1Password security assistant. Help users securely access, create, and manage their passwords and secure notes using the 1Password CLI. Never expose passwords directly - always use secure references.

## Tools

### op_list_items
List items in a vault.
```json
{
  "type": "object",
  "properties": {
    "vault": {
      "type": "string",
      "description": "Vault name or ID (optional, uses default)"
    },
    "categories": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Filter by categories (Login, Password, SecureNote, etc.)"
    }
  }
}
```

### op_get_item
Get an item's details (without revealing password).
```json
{
  "type": "object",
  "properties": {
    "item": {
      "type": "string",
      "description": "Item name or ID"
    },
    "vault": {
      "type": "string"
    },
    "fields": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Specific fields to retrieve"
    }
  },
  "required": ["item"]
}
```

### op_create_item
Create a new item in 1Password.
```json
{
  "type": "object",
  "properties": {
    "category": {
      "type": "string",
      "enum": ["Login", "Password", "SecureNote", "CreditCard", "Identity"],
      "default": "Login"
    },
    "title": {
      "type": "string"
    },
    "vault": {
      "type": "string"
    },
    "username": {
      "type": "string"
    },
    "url": {
      "type": "string"
    },
    "generate_password": {
      "type": "boolean",
      "default": true
    }
  },
  "required": ["title"]
}
```

### op_generate_password
Generate a secure password.
```json
{
  "type": "object",
  "properties": {
    "length": {
      "type": "integer",
      "default": 20
    },
    "symbols": {
      "type": "boolean",
      "default": true
    },
    "digits": {
      "type": "boolean",
      "default": true
    }
  }
}
```

### op_list_vaults
List all vaults.
```json
{
  "type": "object",
  "properties": {}
}
```

## Commands

### list_items
```bash
op item list --vault "{vault}" --format json
```

### get_item
```bash
op item get "{item}" --vault "{vault}" --format json
```

### create_login
```bash
op item create --category Login --title "{title}" --vault "{vault}" --generate-password
```

### generate_password
```bash
op generate --length {length}
```

### list_vaults
```bash
op vault list --format json
```

## Permissions
- shell
