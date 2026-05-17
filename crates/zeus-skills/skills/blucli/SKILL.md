# blucli

Bluetooth device management CLI for macOS and Linux.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Bluetooth management assistant. Help users discover, pair, connect, and manage Bluetooth devices using system Bluetooth tools.

## Tools

### bt_list
List paired Bluetooth devices.
```json
{
  "type": "object",
  "properties": {}
}
```

### bt_scan
Scan for nearby Bluetooth devices.
```json
{
  "type": "object",
  "properties": {
    "duration": {
      "type": "integer",
      "default": 10,
      "description": "Scan duration in seconds"
    }
  }
}
```

### bt_pair
Pair with a Bluetooth device.
```json
{
  "type": "object",
  "properties": {
    "address": {
      "type": "string",
      "description": "Device MAC address"
    }
  },
  "required": ["address"]
}
```

### bt_connect
Connect to a paired device.
```json
{
  "type": "object",
  "properties": {
    "address": {
      "type": "string"
    }
  },
  "required": ["address"]
}
```

### bt_disconnect
Disconnect from a device.
```json
{
  "type": "object",
  "properties": {
    "address": {
      "type": "string"
    }
  },
  "required": ["address"]
}
```

### bt_remove
Remove/unpair a device.
```json
{
  "type": "object",
  "properties": {
    "address": {
      "type": "string"
    }
  },
  "required": ["address"]
}
```

### bt_info
Get device information.
```json
{
  "type": "object",
  "properties": {
    "address": {
      "type": "string"
    }
  },
  "required": ["address"]
}
```

### bt_power
Toggle Bluetooth power.
```json
{
  "type": "object",
  "properties": {
    "state": {
      "type": "string",
      "enum": ["on", "off", "toggle"]
    }
  },
  "required": ["state"]
}
```

## Commands

### list_macos
```bash
system_profiler SPBluetoothDataType 2>/dev/null | grep -A 5 "Connected:"
```

### scan_macos
```bash
blueutil --inquiry {duration}
```

### connect_macos
```bash
blueutil --connect "{address}"
```

### disconnect_macos
```bash
blueutil --disconnect "{address}"
```

### power_macos
```bash
blueutil --power {state}
```

### list_linux
```bash
bluetoothctl devices
```

### scan_linux
```bash
bluetoothctl --timeout {duration} scan on
```

### connect_linux
```bash
bluetoothctl connect "{address}"
```

### disconnect_linux
```bash
bluetoothctl disconnect "{address}"
```

## Permissions
- shell
