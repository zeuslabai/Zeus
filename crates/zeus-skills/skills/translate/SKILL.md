# translate

Translate text between languages using multiple backends (LibreTranslate, DeepL, Google Cloud Translation).

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a translation assistant. Help users translate text between languages, detect languages, and get bilingual comparisons. Use the configured translation backend. When no target language is specified, default to English. For long texts, preserve paragraph structure. Show both original and translated text when helpful.

## Tools

### translate_text
Translate text to a target language.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string",
      "description": "Text to translate"
    },
    "target": {
      "type": "string",
      "description": "Target language code (e.g. 'en', 'es', 'fr', 'de', 'ja', 'zh')",
      "default": "en"
    },
    "source": {
      "type": "string",
      "description": "Source language code (auto-detect if omitted)"
    },
    "backend": {
      "type": "string",
      "enum": ["libre", "deepl", "google"],
      "default": "libre"
    }
  },
  "required": ["text", "target"]
}
```

### translate_detect
Detect the language of input text.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string"
    }
  },
  "required": ["text"]
}
```

### translate_languages
List available languages for the active backend.
```json
{
  "type": "object",
  "properties": {
    "backend": {
      "type": "string",
      "enum": ["libre", "deepl", "google"],
      "default": "libre"
    }
  }
}
```

### translate_file
Translate a text file to a target language.
```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Path to the text file"
    },
    "target": {
      "type": "string",
      "description": "Target language code"
    },
    "source": {
      "type": "string"
    },
    "output_path": {
      "type": "string",
      "description": "Output file path (default: input_path.translated.ext)"
    }
  },
  "required": ["path", "target"]
}
```

## Commands

### libre_translate
```bash
curl -s -X POST "http://localhost:5000/translate" \
  -H "Content-Type: application/json" \
  -d '{"q": "{text}", "source": "{source}", "target": "{target}"}'
```

### deepl_translate
```bash
curl -s -X POST "https://api-free.deepl.com/v2/translate" \
  -H "Authorization: DeepL-Auth-Key $DEEPL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"text": ["{text}"], "target_lang": "{target}"}'
```

### detect
```bash
curl -s -X POST "http://localhost:5000/detect" \
  -H "Content-Type: application/json" \
  -d '{"q": "{text}"}'
```

## Environment
- DEEPL_API_KEY (optional, for DeepL backend)
- GOOGLE_TRANSLATE_API_KEY (optional, for Google backend)
- LIBRETRANSLATE_URL (optional, default http://localhost:5000)

## Permissions
- network
- file_read
