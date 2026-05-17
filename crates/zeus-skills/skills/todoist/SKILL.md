# todoist

Manage tasks, projects, and labels in Todoist via the REST API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Todoist task management assistant. Help users create, organize, and complete tasks across projects. Use natural language due dates when creating tasks (e.g. "tomorrow", "next Monday"). Show tasks grouped by project or due date. Confirm before deleting projects or completing many tasks at once.

## Tools

### todoist_list_tasks
List tasks with optional filters.
```json
{
  "type": "object",
  "properties": {
    "project_id": {
      "type": "string",
      "description": "Filter by project ID"
    },
    "filter": {
      "type": "string",
      "description": "Todoist filter string (e.g. 'today', 'overdue', 'p1', '#Work')"
    },
    "label": {
      "type": "string",
      "description": "Filter by label name"
    }
  }
}
```

### todoist_create_task
Create a new task.
```json
{
  "type": "object",
  "properties": {
    "content": {
      "type": "string",
      "description": "Task title"
    },
    "description": {
      "type": "string"
    },
    "project_id": {
      "type": "string"
    },
    "due_string": {
      "type": "string",
      "description": "Natural language due date (e.g. 'tomorrow', 'every Monday')"
    },
    "priority": {
      "type": "integer",
      "enum": [1, 2, 3, 4],
      "description": "1=normal, 4=urgent"
    },
    "labels": {
      "type": "array",
      "items": {"type": "string"}
    },
    "parent_id": {
      "type": "string",
      "description": "Parent task ID for subtasks"
    }
  },
  "required": ["content"]
}
```

### todoist_complete_task
Mark a task as complete.
```json
{
  "type": "object",
  "properties": {
    "task_id": {
      "type": "string"
    }
  },
  "required": ["task_id"]
}
```

### todoist_update_task
Update an existing task.
```json
{
  "type": "object",
  "properties": {
    "task_id": {
      "type": "string"
    },
    "content": {
      "type": "string"
    },
    "description": {
      "type": "string"
    },
    "due_string": {
      "type": "string"
    },
    "priority": {
      "type": "integer"
    },
    "labels": {
      "type": "array",
      "items": {"type": "string"}
    }
  },
  "required": ["task_id"]
}
```

### todoist_list_projects
List all projects.
```json
{
  "type": "object",
  "properties": {}
}
```

### todoist_create_project
Create a new project.
```json
{
  "type": "object",
  "properties": {
    "name": {
      "type": "string"
    },
    "color": {
      "type": "string",
      "description": "Color name (e.g. 'berry_red', 'blue', 'green')"
    },
    "parent_id": {
      "type": "string",
      "description": "Parent project ID for nesting"
    },
    "is_favorite": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["name"]
}
```

## Commands

### list_tasks
```bash
curl -s "https://api.todoist.com/rest/v2/tasks" \
  -H "Authorization: Bearer $TODOIST_API_KEY"
```

### create_task
```bash
curl -s -X POST "https://api.todoist.com/rest/v2/tasks" \
  -H "Authorization: Bearer $TODOIST_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"content": "{content}", "due_string": "{due_string}", "priority": {priority}}'
```

### complete_task
```bash
curl -s -X POST "https://api.todoist.com/rest/v2/tasks/{task_id}/close" \
  -H "Authorization: Bearer $TODOIST_API_KEY"
```

### list_projects
```bash
curl -s "https://api.todoist.com/rest/v2/projects" \
  -H "Authorization: Bearer $TODOIST_API_KEY"
```

## Environment
- TODOIST_API_KEY

## Permissions
- network
