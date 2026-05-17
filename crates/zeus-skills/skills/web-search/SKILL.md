# web-search

Search the web using multiple engines (SearXNG, Brave Search, Tavily, DuckDuckGo).

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a web search assistant. Help users find information on the web by running searches, summarizing results, and following up on promising links. Cite sources with URLs. Use multiple search queries to triangulate answers when needed. Prefer SearXNG for privacy, Brave for general search, and Tavily for AI-optimized results.

## Tools

### web_search
Execute a web search query.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query"
    },
    "engine": {
      "type": "string",
      "enum": ["searxng", "brave", "tavily", "ddg"],
      "default": "searxng"
    },
    "max_results": {
      "type": "integer",
      "default": 10
    },
    "category": {
      "type": "string",
      "enum": ["general", "news", "images", "science", "it"],
      "default": "general"
    }
  },
  "required": ["query"]
}
```

### web_search_news
Search recent news articles.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "days": {
      "type": "integer",
      "default": 7,
      "description": "Limit to articles from the last N days"
    },
    "max_results": {
      "type": "integer",
      "default": 10
    }
  },
  "required": ["query"]
}
```

### web_search_site
Search within a specific site.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "site": {
      "type": "string",
      "description": "Domain to search within (e.g. 'reddit.com', 'stackoverflow.com')"
    },
    "max_results": {
      "type": "integer",
      "default": 10
    }
  },
  "required": ["query", "site"]
}
```

### web_search_summarize
Search and return an AI-generated summary of top results.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string"
    },
    "depth": {
      "type": "string",
      "enum": ["quick", "thorough"],
      "default": "quick",
      "description": "quick: top 3 results, thorough: top 10 with full text"
    }
  },
  "required": ["query"]
}
```

## Commands

### searxng
```bash
curl -s "http://localhost:8888/search?q={query}&format=json&categories={category}" | python3 -c "import sys,json; [print(f'{r[\"title\"]}\n  {r[\"url\"]}\n  {r.get(\"content\",\"\")[:200]}\n') for r in json.load(sys.stdin).get('results',[])[:10]]"
```

### brave
```bash
curl -s "https://api.search.brave.com/res/v1/web/search?q={query}&count={max_results}" \
  -H "X-Subscription-Token: $BRAVE_SEARCH_API_KEY" \
  -H "Accept: application/json"
```

### tavily
```bash
curl -s -X POST "https://api.tavily.com/search" \
  -H "Content-Type: application/json" \
  -d '{"api_key": "$TAVILY_API_KEY", "query": "{query}", "max_results": {max_results}}'
```

### ddg
```bash
curl -s "https://api.duckduckgo.com/?q={query}&format=json&no_html=1"
```

## Environment
- SEARXNG_URL (optional, default http://localhost:8888)
- BRAVE_SEARCH_API_KEY (optional, for Brave Search)
- TAVILY_API_KEY (optional, for Tavily)

## Permissions
- network
