# oracle

Query knowledge bases, wikis, and reference databases.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a knowledge oracle. Help users find factual information from Wikipedia, Wolfram Alpha, dictionaries, and other reference sources. Provide accurate, well-sourced answers.

## Tools

### oracle_wikipedia
Search and retrieve Wikipedia articles.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "sentences": {
      "type": "integer",
      "default": 3,
      "description": "Number of sentences to return"
    },
    "lang": {
      "type": "string",
      "default": "en"
    }
  },
  "required": ["query"]
}
```

### oracle_wolfram
Query Wolfram Alpha for computations and facts.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Question or computation"
    }
  },
  "required": ["query"]
}
```

### oracle_define
Get word definitions.
```json
{
  "type": "object",
  "properties": {
    "word": {
      "type": "string"
    },
    "lang": {
      "type": "string",
      "default": "en"
    }
  },
  "required": ["word"]
}
```

### oracle_synonym
Find synonyms and antonyms.
```json
{
  "type": "object",
  "properties": {
    "word": {
      "type": "string"
    }
  },
  "required": ["word"]
}
```

### oracle_translate
Translate text.
```json
{
  "type": "object",
  "properties": {
    "text": {
      "type": "string"
    },
    "from": {
      "type": "string",
      "default": "auto"
    },
    "to": {
      "type": "string",
      "default": "en"
    }
  },
  "required": ["text", "to"]
}
```

### oracle_quote
Get a quote by topic or author.
```json
{
  "type": "object",
  "properties": {
    "topic": {
      "type": "string"
    },
    "author": {
      "type": "string"
    }
  }
}
```

### oracle_fact
Get a random fact.
```json
{
  "type": "object",
  "properties": {
    "category": {
      "type": "string",
      "enum": ["trivia", "math", "date", "year"],
      "default": "trivia"
    }
  }
}
```

## Commands

### wikipedia
```bash
curl -s "https://en.wikipedia.org/api/rest_v1/page/summary/{query}" | jq '{title, extract}'
```

### wolfram
```bash
curl -s "https://api.wolframalpha.com/v1/result?i={query}&appid=$WOLFRAM_APP_ID"
```

### define
```bash
curl -s "https://api.dictionaryapi.dev/api/v2/entries/{lang}/{word}" | jq '.[0].meanings[] | {partOfSpeech, definitions: [.definitions[].definition]}'
```

### translate
```bash
curl -s "https://api.mymemory.translated.net/get?q={text}&langpair={from}|{to}" | jq '.responseData.translatedText'
```

### fact
```bash
curl -s "http://numbersapi.com/random/{category}"
```

## Environment
- WOLFRAM_APP_ID

## Permissions
- network
