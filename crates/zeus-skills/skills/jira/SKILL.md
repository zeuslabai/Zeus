# jira

Manage Atlassian Jira issues, sprints, and projects via the Jira REST API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Jira project management assistant. Help users create and manage issues, track sprints, search with JQL, and update workflows. Always confirm before transitions that close or delete issues. Use JQL for complex queries.

## Tools

### jira_search
Search issues using JQL.
```json
{
  "type": "object",
  "properties": {
    "jql": {
      "type": "string",
      "description": "JQL query string (e.g. 'project = ZEUS AND status = Open')"
    },
    "max_results": {
      "type": "integer",
      "default": 20
    },
    "fields": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Fields to return (summary, status, assignee, priority, etc.)"
    }
  },
  "required": ["jql"]
}
```

### jira_get_issue
Get full details of an issue.
```json
{
  "type": "object",
  "properties": {
    "issue_key": {
      "type": "string",
      "description": "Issue key (e.g. ZEUS-123)"
    }
  },
  "required": ["issue_key"]
}
```

### jira_create_issue
Create a new issue.
```json
{
  "type": "object",
  "properties": {
    "project": {
      "type": "string",
      "description": "Project key (e.g. ZEUS)"
    },
    "summary": {
      "type": "string"
    },
    "description": {
      "type": "string"
    },
    "issue_type": {
      "type": "string",
      "enum": ["Bug", "Task", "Story", "Epic", "Sub-task"],
      "default": "Task"
    },
    "priority": {
      "type": "string",
      "enum": ["Highest", "High", "Medium", "Low", "Lowest"],
      "default": "Medium"
    },
    "assignee": {
      "type": "string",
      "description": "Assignee account ID or email"
    },
    "labels": {
      "type": "array",
      "items": {"type": "string"}
    }
  },
  "required": ["project", "summary", "issue_type"]
}
```

### jira_update_issue
Update an existing issue's fields.
```json
{
  "type": "object",
  "properties": {
    "issue_key": {
      "type": "string"
    },
    "summary": {
      "type": "string"
    },
    "description": {
      "type": "string"
    },
    "assignee": {
      "type": "string"
    },
    "priority": {
      "type": "string"
    },
    "labels": {
      "type": "array",
      "items": {"type": "string"}
    }
  },
  "required": ["issue_key"]
}
```

### jira_transition
Transition an issue to a new status.
```json
{
  "type": "object",
  "properties": {
    "issue_key": {
      "type": "string"
    },
    "transition": {
      "type": "string",
      "description": "Transition name (e.g. 'In Progress', 'Done', 'To Do')"
    }
  },
  "required": ["issue_key", "transition"]
}
```

### jira_add_comment
Add a comment to an issue.
```json
{
  "type": "object",
  "properties": {
    "issue_key": {
      "type": "string"
    },
    "body": {
      "type": "string"
    }
  },
  "required": ["issue_key", "body"]
}
```

### jira_list_sprints
List sprints for a board.
```json
{
  "type": "object",
  "properties": {
    "board_id": {
      "type": "integer"
    },
    "state": {
      "type": "string",
      "enum": ["active", "closed", "future"],
      "default": "active"
    }
  },
  "required": ["board_id"]
}
```

## Commands

### search
```bash
curl -s -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H "Content-Type: application/json" \
  "$JIRA_BASE_URL/rest/api/3/search?jql={jql}&maxResults={max_results}"
```

### get_issue
```bash
curl -s -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  "$JIRA_BASE_URL/rest/api/3/issue/{issue_key}"
```

### create_issue
```bash
curl -s -X POST -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H "Content-Type: application/json" \
  "$JIRA_BASE_URL/rest/api/3/issue" \
  -d '{"fields":{"project":{"key":"{project}"},"summary":"{summary}","issuetype":{"name":"{issue_type}"}}}'
```

## Environment
- JIRA_BASE_URL
- JIRA_EMAIL
- JIRA_API_TOKEN

## Permissions
- network
