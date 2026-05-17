# sherpa-onnx-tts

Local text-to-speech using sherpa-onnx models.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a local text-to-speech assistant using sherpa-onnx. Help users convert text to speech with various voices and languages, all running locally without cloud dependencies.

## Tools

### tts_speak
Convert text to speech and play.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string"
    },
    "voice": {
      "type": "string",
      "description": "Voice model name"
    },
    "speed": {
      "type": "number",
      "default": 1.0,
      "description": "Speech speed multiplier"
    }
  },
  "required": ["text"]
}
```

### tts_generate
Generate audio file from text.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string"
    },
    "output": {
      "type": "string",
      "description": "Output audio file path"
    },
    "voice": {
      "type": "string"
    },
    "format": {
      "type": "string",
      "enum": ["wav", "mp3", "ogg"],
      "default": "wav"
    },
    "speed": {
      "type": "number",
      "default": 1.0
    }
  },
  "required": ["text", "output"]
}
```

### tts_list_voices
List available voice models.
```json
{
  "type": "object",
  "properties": {
    "language": {
      "type": "string",
      "description": "Filter by language code"
    }
  }
}
```

### tts_download_voice
Download a new voice model.
```json
{
  "type": "object",
  "properties": {
    "model": {
      "type": "string",
      "description": "Model identifier"
    }
  },
  "required": ["model"]
}
```

### tts_ssml
Generate speech from SSML markup.
```json
{
  "type": "object",
  "properties": {
    "ssml": {
      "type": "string",
      "description": "SSML markup"
    },
    "output": {
      "type": "string"
    }
  },
  "required": ["ssml", "output"]
}
```

## Commands

### speak
```bash
sherpa-onnx-offline-tts \
  --vits-model=$SHERPA_MODEL_PATH/model.onnx \
  --vits-tokens=$SHERPA_MODEL_PATH/tokens.txt \
  --output-filename=/tmp/tts_output.wav \
  --speed={speed} \
  "{text}" && \
afplay /tmp/tts_output.wav
```

### generate
```bash
sherpa-onnx-offline-tts \
  --vits-model=$SHERPA_MODEL_PATH/model.onnx \
  --vits-tokens=$SHERPA_MODEL_PATH/tokens.txt \
  --output-filename="{output}" \
  --speed={speed} \
  "{text}"
```

### list_voices
```bash
ls -1 ~/.local/share/sherpa-onnx/tts/
```

## Environment
- SHERPA_MODEL_PATH

## Permissions
- shell
- filesystem
- audio
