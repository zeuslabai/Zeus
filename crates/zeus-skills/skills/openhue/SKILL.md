# openhue

Control Philips Hue smart lights via the Hue Bridge API.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a smart lighting assistant for Philips Hue. Help users control their lights, create scenes, set schedules, and manage rooms and zones. Use the Hue Bridge API for seamless home automation.

## Tools

### hue_list_lights
List all available lights.
```json
{
  "type": "object",
  "properties": {}
}
```

### hue_set_light
Control a specific light.
```json
{
  "type": "object",
  "properties": {
    "light_id": {
      "type": "string",
      "description": "Light ID or name"
    },
    "on": {
      "type": "boolean",
      "description": "Turn on/off"
    },
    "brightness": {
      "type": "integer",
      "minimum": 1,
      "maximum": 254,
      "description": "Brightness level"
    },
    "color": {
      "type": "string",
      "description": "Color name or hex code"
    },
    "temperature": {
      "type": "integer",
      "minimum": 153,
      "maximum": 500,
      "description": "Color temperature (mireds)"
    }
  },
  "required": ["light_id"]
}
```

### hue_list_rooms
List all rooms and zones.
```json
{
  "type": "object",
  "properties": {}
}
```

### hue_set_room
Control all lights in a room.
```json
{
  "type": "object",
  "properties": {
    "room_id": {
      "type": "string"
    },
    "on": {
      "type": "boolean"
    },
    "brightness": {
      "type": "integer",
      "minimum": 1,
      "maximum": 254
    },
    "scene": {
      "type": "string",
      "description": "Scene name to activate"
    }
  },
  "required": ["room_id"]
}
```

### hue_list_scenes
List available scenes.
```json
{
  "type": "object",
  "properties": {
    "room_id": {
      "type": "string",
      "description": "Filter by room"
    }
  }
}
```

### hue_activate_scene
Activate a scene.
```json
{
  "type": "object",
  "properties": {
    "scene_id": {
      "type": "string"
    }
  },
  "required": ["scene_id"]
}
```

### hue_all_off
Turn off all lights.
```json
{
  "type": "object",
  "properties": {}
}
```

## Commands

### list_lights
```bash
curl -s "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/lights" | jq
```

### set_light
```bash
curl -s -X PUT "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/lights/{light_id}/state" \
  -d '{"on": {on}, "bri": {brightness}}'
```

### list_rooms
```bash
curl -s "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/groups" | jq
```

### list_scenes
```bash
curl -s "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/scenes" | jq
```

### activate_scene
```bash
curl -s -X PUT "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/groups/0/action" \
  -d '{"scene": "{scene_id}"}'
```

### all_off
```bash
curl -s -X PUT "http://$HUE_BRIDGE_IP/api/$HUE_USERNAME/groups/0/action" -d '{"on": false}'
```

## Environment
- HUE_BRIDGE_IP
- HUE_USERNAME

## Permissions
- network
