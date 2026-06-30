# Tools

Zeus ships with 212 tools across 22 categories. Tools are the actions Zeus can take — reading files, running commands, searching the web, controlling your Mac, and more.

## Using Tools from the CLI

Execute any tool directly:

```bash
zeus tool <tool_name> '<json_args>'
```

### Examples

```bash
# Read a file
zeus tool read_file '{"path":"README.md"}'

# Write a file
zeus tool write_file '{"path":"/tmp/hello.txt","content":"Hello, World!"}'

# Edit a file (search and replace)
zeus tool edit_file '{"path":"src/main.rs","search":"old_text","replace":"new_text"}'

# List directory contents
zeus tool list_dir '{"path":"."}'

# Run a shell command
zeus tool shell '{"command":"ls -la"}'

# Fetch a web page
zeus tool web_fetch '{"url":"https://example.com"}'

# Search the web
zeus tool web_search '{"query":"Rust async programming"}'
```

## Using Tools in Chat

You don't need to call tools explicitly in chat — Zeus decides when to use them:

```bash
zeus chat "What's in my current directory?"
# Zeus will use list_dir automatically

zeus chat "Create a Python script that prints fibonacci numbers"
# Zeus will use write_file to create the script
```

## Tool Categories

### Core (8 tools)
The foundation. Available everywhere.

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents |
| `write_file` | Create or overwrite a file |
| `edit_file` | Search and replace in a file |
| `list_dir` | List directory contents |
| `shell` | Execute shell commands |
| `web_fetch` | Fetch a URL |
| `spawn` | Launch a background subagent |
| `message` | Send messages to channels |

### System (43 tools)
macOS system control — screenshots, clipboard, volume, Wi-Fi, Bluetooth, notifications.

```bash
zeus tool system_info '{}'
zeus tool screenshot '{}'
zeus tool clipboard_read '{}'
zeus tool clipboard_write '{"content":"copied text"}'
zeus tool volume_set '{"level":50}'
zeus tool notification_send '{"title":"Zeus","message":"Task complete"}'
zeus tool wifi_list '{}'
zeus tool display_brightness '{"level":75}'
```

### Files (13 tools)
File operations beyond basic read/write.

```bash
zeus tool file_search '{"path":".","pattern":"*.rs"}'
zeus tool file_copy '{"source":"a.txt","destination":"b.txt"}'
zeus tool file_metadata '{"path":"README.md"}'
zeus tool find_files '{"path":".","name":"*.toml"}'
zeus tool head_file '{"path":"log.txt","lines":20}'
zeus tool tail_file '{"path":"log.txt","lines":20}'
```

### Git (15 tools)
Full git workflow from Zeus.

```bash
zeus tool git_status '{}'
zeus tool git_log '{"count":5}'
zeus tool git_diff '{}'
zeus tool git_add '{"files":["src/main.rs"]}'
zeus tool git_commit '{"message":"feat: add feature"}'
zeus tool git_push '{}'
zeus tool git_branch_list '{}'
zeus tool git_branch_create '{"name":"feature/my-feature"}'
```

### Safari (14 tools)
Control Safari browser on macOS.

```bash
zeus tool safari_open_url '{"url":"https://github.com"}'
zeus tool safari_get_url '{}'
zeus tool safari_get_tabs '{}'
zeus tool safari_get_page_text '{}'
zeus tool safari_execute_js '{"script":"document.title"}'
```

### Calendar (7), Notes (9), Reminders (8), Contacts (6)
See [[17-macOS-Automation]] for the full macOS automation guide.

### Mail (10), iMessage (8)
Email and messaging from Zeus.

```bash
zeus tool email_send '{"to":"user@example.com","subject":"Hello","body":"Test email"}'
zeus tool imessage_send '{"to":"+1234567890","message":"Hello from Zeus"}'
```

### Music (10)
Control Apple Music.

```bash
zeus tool music_play '{}'
zeus tool music_pause '{}'
zeus tool music_next '{}'
zeus tool music_now_playing '{}'
zeus tool music_search '{"query":"Beatles"}'
```

### Browser CDP (11)
Chrome DevTools Protocol — see [[11-Browser-Automation]].

### UI Automation (15)
Direct UI control — clicks, typing, window management.

```bash
zeus tool ui_click '{"x":100,"y":200}'
zeus tool ui_type '{"text":"Hello"}'
zeus tool activate_app '{"name":"Terminal"}'
zeus tool get_window_bounds '{"app":"Finder"}'
```

### PDF (5)
```bash
zeus tool pdf_extract_text '{"path":"document.pdf"}'
zeus tool pdf_get_metadata '{"path":"document.pdf"}'
zeus tool pdf_merge '{"files":["a.pdf","b.pdf"],"output":"merged.pdf"}'
```

### Network (3)
```bash
zeus tool network_info '{}'
zeus tool ping '{"host":"google.com"}'
zeus tool port_check '{"host":"localhost","port":8080}'
```

### Homebrew (4)
```bash
zeus tool brew_search '{"query":"ffmpeg"}'
zeus tool brew_install '{"package":"jq"}'
zeus tool brew_list '{}'
```

## Listing All Tools

```bash
# From CLI
zeus tool list_dir '{"path":""}' 
# (Zeus will list its own tools if you ask in chat)

# From API
curl http://localhost:3001/v1/tools | python3 -c "
import sys, json
tools = json.load(sys.stdin)
print(f'{len(tools)} tools available')
for t in sorted(tools, key=lambda x: x['name']):
    print(f\"  {t['name']}: {t.get('description','')[:60]}\")
"
```

## Tool Execution via API

```bash
# NOTE: The API requires an {"arguments": {...}} wrapper
curl -X POST http://localhost:3001/v1/tools/list_dir \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"path":"."}}'
```

> ⚠️ `POST /v1/tools/{name}` requires `{"arguments":{...}}` wrapper — a bare body returns 422.

## What's Next

→ [[06-Memory]] — Zeus's memory system
→ [[11-Browser-Automation]] — Chrome automation
→ [[17-macOS-Automation]] — All 193 macOS tools
