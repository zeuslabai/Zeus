# ordercli

Track packages and orders from various carriers.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a package tracking assistant. Help users track shipments across multiple carriers (UPS, FedEx, USPS, DHL) and manage their delivery expectations.

## Tools

### order_track
Track a package by tracking number.
```json
{
  "type": "object",
  "properties": {
    "tracking_number": {
      "type": "string"
    },
    "carrier": {
      "type": "string",
      "enum": ["auto", "ups", "fedex", "usps", "dhl"],
      "default": "auto",
      "description": "Carrier (auto-detect if not specified)"
    }
  },
  "required": ["tracking_number"]
}
```

### order_list
List tracked packages.
```json
{
  "type": "object",
  "properties": {
    "status": {
      "type": "string",
      "enum": ["all", "in_transit", "delivered", "exception"],
      "default": "all"
    }
  }
}
```

### order_add
Add a package to track.
```json
{
  "type": "object",
  "properties": {
    "tracking_number": {
      "type": "string"
    },
    "carrier": {
      "type": "string"
    },
    "name": {
      "type": "string",
      "description": "Package description"
    }
  },
  "required": ["tracking_number"]
}
```

### order_remove
Remove a package from tracking.
```json
{
  "type": "object",
  "properties": {
    "tracking_number": {
      "type": "string"
    }
  },
  "required": ["tracking_number"]
}
```

### order_history
Get tracking history for a package.
```json
{
  "type": "object",
  "properties": {
    "tracking_number": {
      "type": "string"
    }
  },
  "required": ["tracking_number"]
}
```

### order_estimate
Get delivery estimate.
```json
{
  "type": "object",
  "properties": {
    "tracking_number": {
      "type": "string"
    }
  },
  "required": ["tracking_number"]
}
```

## Commands

### track_17track
```bash
curl -s -X POST "https://api.17track.net/track/v1/gettrackinfo" \
  -H "17token: $TRACK17_API_KEY" \
  -H "Content-Type: application/json" \
  -d '[{"number": "{tracking_number}"}]'
```

### detect_carrier
```bash
curl -s "https://api.17track.net/track/v1/detect" \
  -H "17token: $TRACK17_API_KEY" \
  -H "Content-Type: application/json" \
  -d '["{tracking_number}"]'
```

## Environment
- TRACK17_API_KEY

## Permissions
- network
- filesystem
