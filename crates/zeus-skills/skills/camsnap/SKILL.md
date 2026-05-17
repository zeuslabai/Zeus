# camsnap

Capture images from webcams and IP cameras using ffmpeg or imagesnap.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a camera capture assistant. Help users take snapshots from webcams, IP cameras, and other video sources using command-line tools like imagesnap and ffmpeg.

## Tools

### cam_list
List available cameras.
```json
{
  "type": "object",
  "properties": {}
}
```

### cam_snap
Capture a single frame.
```json
{
  "type": "object",
  "properties": {
    "device": {
      "type": "string",
      "description": "Camera device name or IP stream URL"
    },
    "output": {
      "type": "string",
      "description": "Output file path"
    },
    "warmup": {
      "type": "number",
      "default": 0.5,
      "description": "Warmup time in seconds"
    }
  },
  "required": ["output"]
}
```

### cam_timelapse
Capture multiple frames for timelapse.
```json
{
  "type": "object",
  "properties": {
    "device": {
      "type": "string"
    },
    "output_dir": {
      "type": "string",
      "description": "Output directory"
    },
    "interval": {
      "type": "number",
      "default": 1,
      "description": "Seconds between captures"
    },
    "count": {
      "type": "integer",
      "default": 10,
      "description": "Number of frames"
    }
  },
  "required": ["output_dir"]
}
```

### cam_stream_snap
Capture from an RTSP/HTTP stream.
```json
{
  "type": "object",
  "properties": {
    "url": {
      "type": "string",
      "description": "Stream URL (rtsp://, http://)"
    },
    "output": {
      "type": "string"
    }
  },
  "required": ["url", "output"]
}
```

### cam_record
Record video from camera.
```json
{
  "type": "object",
  "properties": {
    "device": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "duration": {
      "type": "integer",
      "description": "Duration in seconds"
    }
  },
  "required": ["output", "duration"]
}
```

## Commands

### list
```bash
imagesnap -l
```

### snap
```bash
imagesnap -d "{device}" -w {warmup} "{output}"
```

### snap_default
```bash
imagesnap -w 0.5 "{output}"
```

### stream_snap
```bash
ffmpeg -y -i "{url}" -frames:v 1 -q:v 2 "{output}"
```

### record
```bash
ffmpeg -f avfoundation -framerate 30 -i "{device}" -t {duration} "{output}"
```

### timelapse
```bash
for i in $(seq 1 {count}); do imagesnap -d "{device}" "{output_dir}/frame_$i.jpg"; sleep {interval}; done
```

## Permissions
- shell
- filesystem
- camera
