# Browser Automation

Zeus controls Chrome via the Chrome DevTools Protocol (CDP). Navigate pages, click elements, type text, take screenshots, execute JavaScript, and intercept network traffic — all from chat or the API.

## Prerequisites

Start Chrome with remote debugging enabled:

```bash
# macOS
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
  --remote-debugging-port=9222

# Or launch with a specific profile
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
  --remote-debugging-port=9222 \
  --user-data-dir=/tmp/chrome-zeus
```

Zeus connects to `http://localhost:9222` by default.

## Browser Tools (11)

| Tool | Description |
|------|-------------|
| `browser_navigate` | Navigate to a URL |
| `browser_click` | Click an element by CSS selector |
| `browser_type` | Type text into a focused element |
| `browser_screenshot` | Take a screenshot (returns base64 or saves to file) |
| `browser_execute_js` | Run JavaScript in the page |
| `browser_get_text` | Get text content of the page or an element |
| `browser_console_logs` | Get browser console output |
| `browser_network_intercept` | Monitor network requests |
| `browser_performance_metrics` | Page performance data |
| `browser_page_snapshot` | Get page HTML snapshot |
| `browser_list_tabs` | List all open tabs |

## Examples

### Navigate and Screenshot

```bash
zeus tool browser_navigate '{"url":"https://github.com"}'
zeus tool browser_screenshot '{"output":"/tmp/github.png"}'
```

### Click and Type

```bash
# Click a search button
zeus tool browser_click '{"selector":"#search-button"}'

# Type into a form field
zeus tool browser_type '{"selector":"input[name=q]","text":"Zeus AI"}'
```

### Execute JavaScript

```bash
zeus tool browser_execute_js '{"expression":"document.title"}'
zeus tool browser_execute_js '{"expression":"document.querySelectorAll(\"a\").length"}'
```

### Get Page Content

```bash
# Full page text
zeus tool browser_get_text '{}'

# Specific element
zeus tool browser_get_text '{"selector":"#main-content"}'
```

## In Chat

Ask Zeus to interact with websites:

```bash
zeus chat "Go to github.com and take a screenshot"
zeus chat "Navigate to example.com and get the page title"
zeus chat "Fill in the search box on google.com with 'Zeus AI' and screenshot the results"
```

Zeus will chain multiple browser tools to complete the task.

## Stealth Mode

For sites with bot detection (Cloudflare, reCAPTCHA):

```bash
zeus tool browser_enable_stealth '{}'
# Then navigate — uses realistic user agent and anti-detection measures
```

## Tips

- **Selectors**: Use CSS selectors for clicking and typing (`#id`, `.class`, `[name=value]`)
- **Wait for load**: Navigate first, then interact — Zeus handles page load timing
- **Multi-tab**: Use `browser_list_tabs` to see all tabs, `browser_new_tab` to open new ones
- **Debug**: `browser_console_logs` shows JavaScript errors and console output

## What's Next

→ [[05-Tools]] — Full tool reference
→ [[17-macOS-Automation]] — macOS automation tools
