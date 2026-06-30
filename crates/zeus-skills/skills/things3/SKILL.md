# things3

Interact with Things 3 task manager on macOS via URL schemes and AppleScript.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Things 3 task management assistant. Help users create todos, manage projects, organize tasks with areas and tags, and stay productive using the Things 3 app on macOS.

## Tools

### things_add_todo
Create a new todo in Things.
```json
{
  "type": "object",
  "properties": {
    "title": {
      "type": "string",
      "description": "Todo title"
    },
    "notes": {
      "type": "string",
      "description": "Additional notes"
    },
    "when": {
      "type": "string",
      "description": "Date (today, tomorrow, evening, anytime, someday, or YYYY-MM-DD)"
    },
    "deadline": {
      "type": "string",
      "description": "Deadline date (YYYY-MM-DD)"
    },
    "tags": {
      "type": "array",
      "items": {"type": "string"}
    },
    "list": {
      "type": "string",
      "description": "Project or area name"
    },
    "checklist": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Checklist items"
    }
  },
  "required": ["title"]
}
```

### things_add_project
Create a new project.
```json
{
  "type": "object",
  "properties": {
    "title": {
      "type": "string"
    },
    "notes": {
      "type": "string"
    },
    "area": {
      "type": "string",
      "description": "Area to add project to"
    },
    "when": {
      "type": "string"
    },
    "deadline": {
      "type": "string"
    },
    "tags": {
      "type": "array",
      "items": {"type": "string"}
    },
    "todos": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Initial todos for the project"
    }
  },
  "required": ["title"]
}
```

### things_show
Show a specific list in Things.
```json
{
  "type": "object",
  "properties": {
    "list": {
      "type": "string",
      "enum": ["inbox", "today", "upcoming", "anytime", "someday", "logbook", "trash"],
      "default": "today"
    }
  }
}
```

### things_search
Search for todos and projects.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    }
  },
  "required": ["query"]
}
```

### things_complete
Mark a todo as complete.
```json
{
  "type": "object",
  "properties": {
    "id": {
      "type": "string",
      "description": "Todo ID"
    }
  },
  "required": ["id"]
}
```

## Commands

### add_todo
```bash
open "things:///add?title={title}&notes={notes}&when={when}&deadline={deadline}&list={list}"
```

### add_project
```bash
open "things:///add-project?title={title}&notes={notes}&area={area}&when={when}"
```

### show_list
```bash
open "things:///show?id={list}"
```

### search
```bash
open "things:///search?query={query}"
```

## Permissions
- applescript
- url_scheme
