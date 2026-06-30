# gemini

Google Gemini API integration for multimodal AI capabilities.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a Google Gemini integration assistant. Help users leverage Gemini's multimodal capabilities for text generation, image analysis, and code generation.

## Tools

### gemini_generate
Generate text with Gemini.
```json
{
  "type": "object",
  "properties": {
    "prompt": {
      "type": "string"
    },
    "model": {
      "type": "string",
      "enum": ["gemini-pro", "gemini-pro-vision", "gemini-1.5-pro", "gemini-1.5-flash"],
      "default": "gemini-1.5-flash"
    },
    "max_tokens": {
      "type": "integer",
      "default": 1024
    },
    "temperature": {
      "type": "number",
      "default": 0.7
    }
  },
  "required": ["prompt"]
}
```

### gemini_analyze_image
Analyze an image with Gemini Vision.
```json
{
  "type": "object",
  "properties": {
    "image": {
      "type": "string",
      "description": "Path to image file or URL"
    },
    "prompt": {
      "type": "string",
      "default": "Describe this image in detail"
    }
  },
  "required": ["image"]
}
```

### gemini_chat
Multi-turn chat with Gemini.
```json
{
  "type": "object",
  "properties": {
    "message": {
      "type": "string"
    },
    "session_id": {
      "type": "string",
      "description": "Chat session ID for context"
    }
  },
  "required": ["message"]
}
```

### gemini_embed
Generate embeddings with Gemini.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string"
    },
    "model": {
      "type": "string",
      "default": "embedding-001"
    }
  },
  "required": ["text"]
}
```

### gemini_code
Generate or explain code.
```json
{
  "type": "object",
  "properties": {
    "prompt": {
      "type": "string"
    },
    "language": {
      "type": "string"
    },
    "task": {
      "type": "string",
      "enum": ["generate", "explain", "review", "fix"],
      "default": "generate"
    }
  },
  "required": ["prompt"]
}
```

## Commands

### generate
```bash
curl -s -X POST "https://generativelanguage.googleapis.com/v1/models/{model}:generateContent?key=$GOOGLE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"parts": [{"text": "{prompt}"}]}], "generationConfig": {"maxOutputTokens": {max_tokens}, "temperature": {temperature}}}'
```

### analyze_image
```bash
curl -s -X POST "https://generativelanguage.googleapis.com/v1/models/gemini-pro-vision:generateContent?key=$GOOGLE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"contents": [{"parts": [{"text": "{prompt}"}, {"inline_data": {"mime_type": "image/jpeg", "data": "'$(base64 -i "{image}")'"}}]}]}'
```

### embed
```bash
curl -s -X POST "https://generativelanguage.googleapis.com/v1/models/{model}:embedContent?key=$GOOGLE_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"content": {"parts": [{"text": "{text}"}]}}'
```

## Environment
- GOOGLE_API_KEY

## Permissions
- network
