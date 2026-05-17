# macOS Automation — Talos

Zeus includes 193 macOS automation tools powered by AppleScript and system APIs. These let Zeus control Calendar, Notes, Reminders, Contacts, Safari, Mail, Music, Finder, Bluetooth, Wi-Fi, and more.

## Enable Talos

```toml
# ~/.zeus/config.toml
[talos]
calendar = true
notes = true
reminders = true
contacts = true
browser = true
system = true
network = true
```

## Permissions

macOS requires accessibility and automation permissions. On first use of certain tools, you'll see permission prompts:

1. **Accessibility** — System Settings → Privacy & Security → Accessibility → Enable for Terminal/iTerm
2. **Automation** — System Settings → Privacy & Security → Automation → Allow Terminal to control apps

## System Tools (43)

### System Info & Control

```bash
zeus tool system_info '{}'           # Hardware and OS details
zeus tool process_list '{}'          # Running processes
zeus tool kill_process '{"pid":1234}'
zeus tool disk_space '{}'
zeus tool env_vars '{}'
```

### Display & Audio

```bash
zeus tool screenshot '{}'                       # Full screen
zeus tool screenshot '{"path":"/tmp/shot.png"}' # Save to file
zeus tool volume_get '{}'
zeus tool volume_set '{"level":50}'             # 0-100
zeus tool display_brightness '{"level":75}'
zeus tool appearance_toggle '{}'                # Light ↔ Dark mode
```

### Clipboard

```bash
zeus tool clipboard_read '{}'
zeus tool clipboard_write '{"content":"Hello from Zeus"}'
```

### Notifications

```bash
zeus tool notification_send '{"title":"Zeus","message":"Task complete"}'
```

### Wi-Fi

```bash
zeus tool wifi_list '{}'
zeus tool wifi_connect '{"network":"MyWiFi","password":"secret"}'
```

### Application Control

```bash
zeus tool open_app '{"name":"Finder"}'
zeus tool quit_app '{"name":"Preview"}'
zeus tool focus_mode '{}'
```

## Calendar (7 tools)

```bash
# List today's events
zeus tool calendar_get_today '{}'

# List events in a date range
zeus tool calendar_list_events '{"from":"2026-03-01","to":"2026-03-07"}'

# Create an event
zeus tool calendar_create_event '{
  "title":"Team Standup",
  "start":"2026-03-15T09:00:00",
  "end":"2026-03-15T09:30:00",
  "calendar":"Work"
}'

# Delete an event
zeus tool calendar_delete_event '{"title":"Team Standup","date":"2026-03-15"}'
```

Or just ask in chat:

```
"What's on my calendar today?"
"Schedule a meeting with Alex tomorrow at 2pm for 30 minutes"
```

## Notes (9 tools)

```bash
# List all notes
zeus tool notes_list '{}'

# Create a note
zeus tool notes_create '{"title":"Meeting Notes","body":"Discussed Q2 goals"}'

# Read a note
zeus tool notes_read '{"title":"Meeting Notes"}'

# Search notes
zeus tool notes_search '{"query":"Q2 goals"}'

# Append to a note
zeus tool notes_append '{"title":"Meeting Notes","text":"\n- Action item: review budget"}'

# List folders
zeus tool notes_list_folders '{}'
```

## Reminders (8 tools)

```bash
# List reminders
zeus tool reminder_list '{}'

# Create a reminder
zeus tool reminder_create '{
  "title":"Review PR #42",
  "due":"2026-03-15T17:00:00",
  "list":"Work"
}'

# Complete a reminder
zeus tool reminder_complete '{"title":"Review PR #42"}'

# List reminder lists
zeus tool reminder_lists '{}'
```

## Contacts (6 tools)

```bash
zeus tool contacts_search '{"query":"Alice"}'
zeus tool contacts_get_details '{"name":"Alice Smith"}'
zeus tool contacts_create '{"name":"Bob Jones","email":"bob@example.com","phone":"+1234567890"}'
```

## Mail (10 tools)

```bash
# List recent emails
zeus tool mail_list '{"count":10}'

# Send an email
zeus tool email_send '{
  "to":"alice@example.com",
  "subject":"Hello from Zeus",
  "body":"This email was sent by Zeus."
}'

# Search email
zeus tool email_search '{"query":"project update"}'

# Flag / unflag
zeus tool mail_flag '{"subject":"Important","flag":true}'

# Mark as read
zeus tool mail_mark_read '{"subject":"Weekly Report"}'
```

## Music (10 tools)

```bash
zeus tool music_play '{}'
zeus tool music_pause '{}'
zeus tool music_next '{}'
zeus tool music_previous '{}'
zeus tool music_now_playing '{}'
zeus tool music_search '{"query":"Beatles"}'
zeus tool music_set_volume '{"level":40}'
```

Or in chat:

```
"Play some Beatles"
"What song is playing?"
"Turn the music down"
```

## Safari (14 tools)

```bash
zeus tool safari_open_url '{"url":"https://github.com"}'
zeus tool safari_get_url '{}'
zeus tool safari_get_tabs '{}'
zeus tool safari_get_page_text '{}'
zeus tool safari_navigate '{"direction":"back"}'
zeus tool safari_execute_js '{"script":"document.title"}'
```

## iMessage (8 tools)

```bash
zeus tool imessage_send '{"to":"+1234567890","message":"Hello from Zeus"}'
zeus tool imessage_read '{}'
zeus tool imessage_list_conversations '{}'
```

## UI Automation (15 tools)

Direct control of the macOS UI:

```bash
zeus tool ui_click '{"x":500,"y":300}'
zeus tool ui_type '{"text":"Hello World"}'
zeus tool ui_scroll '{"direction":"down","amount":3}'
zeus tool ui_shortcut '{"keys":["cmd","s"]}'              # Cmd+S
zeus tool activate_app '{"name":"Finder"}'
zeus tool get_window_bounds '{"app":"Terminal"}'
zeus tool move_window '{"app":"Terminal","x":100,"y":100}'
zeus tool resize_window '{"app":"Terminal","width":800,"height":600}'
zeus tool minimize_window '{"app":"Preview"}'
zeus tool maximize_window '{"app":"Terminal"}'
```

## PDF (5 tools)

```bash
zeus tool pdf_extract_text '{"path":"document.pdf"}'
zeus tool pdf_get_metadata '{"path":"document.pdf"}'
zeus tool pdf_merge '{"files":["a.pdf","b.pdf"],"output":"merged.pdf"}'
zeus tool pdf_split '{"path":"book.pdf","pages":[1,5]}'
zeus tool pdf_extract_pages '{"path":"book.pdf","pages":[3,7],"output":"extract.pdf"}'
```

## Bluetooth (6 tools)

```bash
zeus tool bluetooth_list '{}'
zeus tool bluetooth_connect '{"device":"AirPods Pro"}'
zeus tool bluetooth_disconnect '{"device":"AirPods Pro"}'
zeus tool bluetooth_power '{"state":"on"}'
```

## Homebrew (4 tools)

```bash
zeus tool brew_search '{"query":"ffmpeg"}'
zeus tool brew_install '{"package":"jq"}'
zeus tool brew_uninstall '{"package":"jq"}'
zeus tool brew_list '{}'
```

## In Chat

You don't need to call tools manually — just ask:

```
"What's on my calendar this week?"
"Send an email to alice@example.com about the project update"
"Create a reminder to review the PR by Friday"
"Take a screenshot and save it to my Desktop"
"Turn the volume down to 30%"
"Connect my AirPods"
```

Zeus will use the appropriate Talos tools automatically.

## What's Next

→ [[05-Tools]] — Full tool reference
→ [[11-Browser-Automation]] — Chrome automation
