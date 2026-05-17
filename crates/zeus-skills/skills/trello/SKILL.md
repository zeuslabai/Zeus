# trello

Manage Trello boards, lists, and cards via the Trello API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Trello project management assistant. Help users manage their boards, create and organize cards, track tasks, and collaborate with team members using the Trello API.

## Tools

### trello_list_boards
List all boards for the authenticated user.
```json
{
  "type": "object",
  "properties": {}
}
```

### trello_get_board
Get a board with its lists and cards.
```json
{
  "type": "object",
  "properties": {
    "board_id": {
      "type": "string",
      "description": "Trello board ID"
    }
  },
  "required": ["board_id"]
}
```

### trello_create_card
Create a new card on a list.
```json
{
  "type": "object",
  "properties": {
    "list_id": {
      "type": "string",
      "description": "List ID to add card to"
    },
    "name": {
      "type": "string",
      "description": "Card title"
    },
    "desc": {
      "type": "string",
      "description": "Card description"
    },
    "due": {
      "type": "string",
      "description": "Due date (ISO 8601)"
    },
    "labels": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Label IDs to attach"
    }
  },
  "required": ["list_id", "name"]
}
```

### trello_move_card
Move a card to a different list.
```json
{
  "type": "object",
  "properties": {
    "card_id": {
      "type": "string"
    },
    "list_id": {
      "type": "string",
      "description": "Destination list ID"
    }
  },
  "required": ["card_id", "list_id"]
}
```

### trello_add_comment
Add a comment to a card.
```json
{
  "type": "object",
  "properties": {
    "card_id": {
      "type": "string"
    },
    "text": {
      "type": "string",
      "description": "Comment text"
    }
  },
  "required": ["card_id", "text"]
}
```

### trello_create_list
Create a new list on a board.
```json
{
  "type": "object",
  "properties": {
    "board_id": {
      "type": "string"
    },
    "name": {
      "type": "string",
      "description": "List name"
    }
  },
  "required": ["board_id", "name"]
}
```

## Commands

### list_boards
```bash
curl -s "https://api.trello.com/1/members/me/boards?key=$TRELLO_API_KEY&token=$TRELLO_TOKEN"
```

### create_card
```bash
curl -s -X POST "https://api.trello.com/1/cards?key=$TRELLO_API_KEY&token=$TRELLO_TOKEN&idList={list_id}&name={name}"
```

## Environment
- TRELLO_API_KEY
- TRELLO_TOKEN

## Permissions
- network
