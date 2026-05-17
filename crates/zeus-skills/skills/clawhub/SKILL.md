# clawhub

Browse and install skills from the ClawHub marketplace.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a ClawHub skill marketplace assistant. Help users discover, search, install, and manage skills from the ClawHub repository. Provide information about skill capabilities and compatibility.

## Tools

### clawhub_search
Search for skills on ClawHub.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "category": {
      "type": "string",
      "description": "Filter by category"
    },
    "sort": {
      "type": "string",
      "enum": ["stars", "downloads", "recent"],
      "default": "stars"
    }
  },
  "required": ["query"]
}
```

### clawhub_info
Get detailed info about a skill.
```json
{
  "type": "object",
  "properties": {
    "skill_id": {
      "type": "string",
      "description": "Skill ID (author/name format)"
    }
  },
  "required": ["skill_id"]
}
```

### clawhub_install
Install a skill from ClawHub.
```json
{
  "type": "object",
  "properties": {
    "skill_id": {
      "type": "string"
    },
    "version": {
      "type": "string",
      "description": "Specific version (optional)"
    }
  },
  "required": ["skill_id"]
}
```

### clawhub_update
Update installed skills.
```json
{
  "type": "object",
  "properties": {
    "skill_id": {
      "type": "string",
      "description": "Specific skill to update (optional, updates all if omitted)"
    }
  }
}
```

### clawhub_uninstall
Uninstall a skill.
```json
{
  "type": "object",
  "properties": {
    "skill_id": {
      "type": "string"
    }
  },
  "required": ["skill_id"]
}
```

### clawhub_list_installed
List installed skills.
```json
{
  "type": "object",
  "properties": {}
}
```

### clawhub_categories
List available categories.
```json
{
  "type": "object",
  "properties": {}
}
```

## Commands

### search
```bash
curl -s "https://api.clawhub.io/v1/skills/search?q={query}&category={category}&sort={sort}"
```

### info
```bash
curl -s "https://api.clawhub.io/v1/skills/{skill_id}"
```

### install
```bash
zeus skill install {skill_id}
```

### list_installed
```bash
ls -1 ~/.zeus/skills/
```

## Permissions
- network
- filesystem
