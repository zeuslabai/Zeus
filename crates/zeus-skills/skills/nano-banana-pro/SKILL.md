# nano-banana-pro

Banana.dev serverless GPU inference for ML models.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Banana.dev ML inference assistant. Help users run machine learning models on serverless GPUs for image generation, text generation, and other AI tasks.

## Tools

### banana_run
Run inference on a Banana model.
```json
{
  "type": "object",
  "properties": {
    "model_key": {
      "type": "string",
      "description": "Banana model key"
    },
    "inputs": {
      "type": "object",
      "description": "Model inputs"
    }
  },
  "required": ["model_key", "inputs"]
}
```

### banana_sdxl
Generate image with SDXL.
```json
{
  "type": "object",
  "properties": {
    "prompt": {
      "type": "string"
    },
    "negative_prompt": {
      "type": "string"
    },
    "width": {
      "type": "integer",
      "default": 1024
    },
    "height": {
      "type": "integer",
      "default": 1024
    },
    "steps": {
      "type": "integer",
      "default": 30
    }
  },
  "required": ["prompt"]
}
```

### banana_whisper
Transcribe audio with Whisper.
```json
{
  "type": "object",
  "properties": {
    "audio_url": {
      "type": "string",
      "description": "URL to audio file"
    },
    "language": {
      "type": "string",
      "default": "en"
    }
  },
  "required": ["audio_url"]
}
```

### banana_llama
Generate text with Llama.
```json
{
  "type": "object",
  "properties": {
    "prompt": {
      "type": "string"
    },
    "max_tokens": {
      "type": "integer",
      "default": 512
    },
    "temperature": {
      "type": "number",
      "default": 0.7
    }
  },
  "required": ["prompt"]
}
```

### banana_status
Check model/job status.
```json
{
  "type": "object",
  "properties": {
    "call_id": {
      "type": "string"
    }
  },
  "required": ["call_id"]
}
```

### banana_list_models
List available models.
```json
{
  "type": "object",
  "properties": {}
}
```

## Commands

### run
```bash
curl -s -X POST "https://api.banana.dev/start/v4/" \
  -H "Content-Type: application/json" \
  -d '{"apiKey": "$BANANA_API_KEY", "modelKey": "{model_key}", "modelInputs": {inputs}}'
```

### status
```bash
curl -s "https://api.banana.dev/check/v4/" \
  -H "Content-Type: application/json" \
  -d '{"apiKey": "$BANANA_API_KEY", "callID": "{call_id}"}'
```

## Environment
- BANANA_API_KEY

## Permissions
- network
