# video-frames

Extract frames and thumbnails from video files using ffmpeg.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a video frame extraction assistant. Help users extract frames, create thumbnails, generate GIFs, and analyze video content using ffmpeg.

## Tools

### video_info
Get video file information.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string",
      "description": "Video file path"
    }
  },
  "required": ["input"]
}
```

### video_extract_frame
Extract a single frame at a timestamp.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "timestamp": {
      "type": "string",
      "description": "Timestamp (HH:MM:SS or seconds)"
    },
    "output": {
      "type": "string"
    }
  },
  "required": ["input", "timestamp", "output"]
}
```

### video_extract_frames
Extract multiple frames at interval.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output_pattern": {
      "type": "string",
      "description": "Output pattern (e.g., 'frame_%04d.jpg')"
    },
    "fps": {
      "type": "number",
      "default": 1,
      "description": "Frames per second to extract"
    },
    "start": {
      "type": "string",
      "description": "Start timestamp"
    },
    "duration": {
      "type": "string",
      "description": "Duration to extract"
    }
  },
  "required": ["input", "output_pattern"]
}
```

### video_thumbnail
Generate video thumbnail/poster.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "position": {
      "type": "string",
      "default": "10%",
      "description": "Position in video (percentage or timestamp)"
    },
    "size": {
      "type": "string",
      "default": "320x240",
      "description": "Thumbnail size (WxH)"
    }
  },
  "required": ["input", "output"]
}
```

### video_to_gif
Create animated GIF from video segment.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "start": {
      "type": "string",
      "description": "Start timestamp"
    },
    "duration": {
      "type": "number",
      "default": 3,
      "description": "Duration in seconds"
    },
    "fps": {
      "type": "integer",
      "default": 10
    },
    "width": {
      "type": "integer",
      "default": 320
    }
  },
  "required": ["input", "output"]
}
```

### video_mosaic
Create a mosaic/contact sheet of frames.
```json
{
  "type": "object",
  "properties": {
    "input": {
      "type": "string"
    },
    "output": {
      "type": "string"
    },
    "columns": {
      "type": "integer",
      "default": 4
    },
    "rows": {
      "type": "integer",
      "default": 4
    }
  },
  "required": ["input", "output"]
}
```

## Commands

### info
```bash
ffprobe -v quiet -print_format json -show_format -show_streams "{input}"
```

### extract_frame
```bash
ffmpeg -y -ss {timestamp} -i "{input}" -frames:v 1 -q:v 2 "{output}"
```

### extract_frames
```bash
ffmpeg -y -i "{input}" -vf "fps={fps}" "{output_pattern}"
```

### thumbnail
```bash
ffmpeg -y -i "{input}" -ss {position} -frames:v 1 -s {size} "{output}"
```

### to_gif
```bash
ffmpeg -y -ss {start} -t {duration} -i "{input}" -vf "fps={fps},scale={width}:-1:flags=lanczos" "{output}"
```

### mosaic
```bash
ffmpeg -y -i "{input}" -vf "select='not(mod(n,$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 {input} | awk '{print int($1/16)}')))'" -vsync vfr -frames:v 16 -q:v 2 "/tmp/mosaic_%02d.jpg" && montage /tmp/mosaic_*.jpg -tile {columns}x{rows} -geometry +2+2 "{output}"
```

## Permissions
- shell
- filesystem
