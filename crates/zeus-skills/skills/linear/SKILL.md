# linear

Manage Linear issues, projects, and cycles via the Linear GraphQL API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Linear project management assistant. Help users create issues, manage cycles, track projects, and search their Linear workspace. Use the GraphQL API for all operations. Present issues with their identifier, title, status, and assignee.

## Tools

### linear_search
Search issues in Linear.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query string"
    },
    "team": {
      "type": "string",
      "description": "Team key to filter by"
    },
    "status": {
      "type": "string",
      "enum": ["Backlog", "Todo", "In Progress", "In Review", "Done", "Canceled"]
    },
    "limit": {
      "type": "integer",
      "default": 20
    }
  },
  "required": ["query"]
}
```

### linear_get_issue
Get full details of an issue by identifier.
```json
{
  "type": "object",
  "properties": {
    "issue_id": {
      "type": "string",
      "description": "Issue identifier (e.g. TEAM-123)"
    }
  },
  "required": ["issue_id"]
}
```

### linear_create_issue
Create a new issue.
```json
{
  "type": "object",
  "properties": {
    "team_id": {
      "type": "string",
      "description": "Team ID or key"
    },
    "title": {
      "type": "string"
    },
    "description": {
      "type": "string",
      "description": "Markdown description"
    },
    "priority": {
      "type": "integer",
      "enum": [0, 1, 2, 3, 4],
      "description": "0=None, 1=Urgent, 2=High, 3=Medium, 4=Low"
    },
    "assignee_id": {
      "type": "string"
    },
    "label_ids": {
      "type": "array",
      "items": {"type": "string"}
    },
    "cycle_id": {
      "type": "string"
    }
  },
  "required": ["team_id", "title"]
}
```

### linear_update_issue
Update an existing issue.
```json
{
  "type": "object",
  "properties": {
    "issue_id": {
      "type": "string"
    },
    "title": {
      "type": "string"
    },
    "description": {
      "type": "string"
    },
    "status": {
      "type": "string"
    },
    "priority": {
      "type": "integer"
    },
    "assignee_id": {
      "type": "string"
    }
  },
  "required": ["issue_id"]
}
```

### linear_list_teams
List all teams in the workspace.
```json
{
  "type": "object",
  "properties": {}
}
```

### linear_list_cycles
List cycles for a team.
```json
{
  "type": "object",
  "properties": {
    "team_id": {
      "type": "string"
    },
    "filter": {
      "type": "string",
      "enum": ["active", "upcoming", "completed"],
      "default": "active"
    }
  },
  "required": ["team_id"]
}
```

## Commands

### graphql
```bash
curl -s -X POST https://api.linear.app/graphql \
  -H "Authorization: $LINEAR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "{query}"}'
```

## Environment
- LINEAR_API_KEY

## Permissions
- network
