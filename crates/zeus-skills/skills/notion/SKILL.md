# notion

Interact with Notion workspaces via the Notion API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Notion workspace assistant. Help users create pages, manage databases, search content, and organize their Notion workspace. Use the Notion API tools to interact with their workspace.

## Tools

### notion_search
Search pages and databases in Notion.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "filter": {
      "type": "string",
      "enum": ["page", "database"],
      "description": "Filter by object type"
    }
  },
  "required": ["query"]
}
```

### notion_create_page
Create a new page in Notion.
```json
{
  "type": "object",
  "properties": {
    "parent_id": {
      "type": "string",
      "description": "Parent page or database ID"
    },
    "title": {
      "type": "string",
      "description": "Page title"
    },
    "content": {
      "type": "string",
      "description": "Page content in markdown"
    }
  },
  "required": ["parent_id", "title"]
}
```

### notion_get_page
Get a page by ID.
```json
{
  "type": "object",
  "properties": {
    "page_id": {
      "type": "string",
      "description": "Notion page ID"
    }
  },
  "required": ["page_id"]
}
```

### notion_update_page
Update a page's properties.
```json
{
  "type": "object",
  "properties": {
    "page_id": {
      "type": "string"
    },
    "properties": {
      "type": "object",
      "description": "Properties to update"
    }
  },
  "required": ["page_id", "properties"]
}
```

### notion_query_database
Query a Notion database.
```json
{
  "type": "object",
  "properties": {
    "database_id": {
      "type": "string"
    },
    "filter": {
      "type": "object",
      "description": "Notion filter object"
    },
    "sorts": {
      "type": "array",
      "description": "Sort configuration"
    }
  },
  "required": ["database_id"]
}
```

### notion_add_database_item
Add an item to a database.
```json
{
  "type": "object",
  "properties": {
    "database_id": {
      "type": "string"
    },
    "properties": {
      "type": "object",
      "description": "Item properties matching database schema"
    }
  },
  "required": ["database_id", "properties"]
}
```

## Commands

### search
```bash
curl -s -X POST 'https://api.notion.com/v1/search' \
  -H "Authorization: Bearer $NOTION_API_KEY" \
  -H "Notion-Version: 2022-06-28" \
  -H "Content-Type: application/json" \
  -d '{"query": "{query}"}'
```

## Environment
- NOTION_API_KEY

## Permissions
- network
