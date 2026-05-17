# eightctl

Control Eight Sleep smart mattress for sleep tracking and temperature.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are an Eight Sleep smart mattress assistant. Help users control their mattress temperature, view sleep data, and manage sleep schedules via the Eight Sleep API.

## Tools

### eight_status
Get current mattress status.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right", "both"],
      "default": "both"
    }
  }
}
```

### eight_set_temp
Set mattress temperature level.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right"]
    },
    "level": {
      "type": "integer",
      "minimum": -100,
      "maximum": 100,
      "description": "Temperature level (-100 to 100)"
    }
  },
  "required": ["side", "level"]
}
```

### eight_turn_on
Turn on the mattress heating/cooling.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right", "both"]
    }
  },
  "required": ["side"]
}
```

### eight_turn_off
Turn off the mattress.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right", "both"]
    }
  },
  "required": ["side"]
}
```

### eight_sleep_data
Get sleep data for a date range.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right"]
    },
    "start_date": {
      "type": "string",
      "description": "Start date (YYYY-MM-DD)"
    },
    "end_date": {
      "type": "string"
    }
  },
  "required": ["side"]
}
```

### eight_schedule
Set a temperature schedule.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right"]
    },
    "schedule": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "time": {"type": "string"},
          "level": {"type": "integer"}
        }
      }
    }
  },
  "required": ["side", "schedule"]
}
```

### eight_alarm
Set a gentle wake alarm.
```json
{
  "type": "object",
  "properties": {
    "side": {
      "type": "string",
      "enum": ["left", "right"]
    },
    "time": {
      "type": "string",
      "description": "Alarm time (HH:MM)"
    },
    "enabled": {
      "type": "boolean",
      "default": true
    }
  },
  "required": ["side", "time"]
}
```

## Commands

### status
```bash
curl -s -H "Authorization: Bearer $EIGHT_SLEEP_TOKEN" \
  "https://client-api.8slp.net/v1/users/me/device-status"
```

### set_temp
```bash
curl -s -X PUT -H "Authorization: Bearer $EIGHT_SLEEP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"side": "{side}", "level": {level}}' \
  "https://client-api.8slp.net/v1/users/me/temperature"
```

### sleep_data
```bash
curl -s -H "Authorization: Bearer $EIGHT_SLEEP_TOKEN" \
  "https://client-api.8slp.net/v1/users/me/intervals?from={start_date}&to={end_date}"
```

## Environment
- EIGHT_SLEEP_TOKEN

## Permissions
- network
