# Zeus — Full Testing Guide

> Current main: `f999545c` · Updated: 2026-03-03

## S20 Phase 1 merges

| Commit | Item |
|--------|------|
| `17dbad91` | MCP server dedup — `POST /v1/mcp/servers` upserts by `id` |
| `36bf0f48` | Ollama redirect — POST preserved through HTTP→HTTPS 301 |
| `06ddb35f` | S20-1 — `agents_dir()` workspace isolation (fixes stale agent list) |
| `c03bd10d` | S20-6 — `POST /v1/agents` upserts by name (prevents bloat) |
| `95d4096b` | S20-2 — `POST /v1/nous/observe` CL-v2 observer hook |
| `f8acf0a7` | S20-4 — 10 slash commands via skill routing in Pantheon |
| `48ad0fb5` | S20-3 — 5 agent persona templates + `GET /v1/personas` + `POST /v1/agents/from-persona/:name` |

## Known Issues (S20 backlog)

| # | Severity | Item |
|---|----------|------|
| ~~S20-1~~ | ~~🐛 Bug~~ | ~~`GET /v1/agents` stale entries~~ — **FIXED** `06ddb35f` + `c03bd10d` |
| S20-2 | 📝 Docs | `POST /v1/tools/{name}` requires `{"arguments":{...}}` wrapper — bare body returns 422. See §5 Tools. |
| S20-3 | 📝 Docs | Channel create uses `channel_type` field, not `type`. See §5 Channels. |

---

## 1. Prerequisites

```bash
# Env vars — copy from ~/.zeus/.env or set manually
export ANTHROPIC_API_KEY=...
export OPENAI_API_KEY=...          # optional, for OpenAI models
export DISCORD_BOT_TOKEN=...       # channels testing
export TELEGRAM_API_ID=...
export TELEGRAM_API_HASH=...
export TELEGRAM_PHONE=...
export SLACK_BOT_TOKEN=...
export SLACK_APP_TOKEN=...
export WHATSAPP_TOKEN=...
export WHATSAPP_PHONE_NUMBER_ID=...
export SIGNAL_CLI_PATH=...         # signal-cli binary
export MATRIX_HOMESERVER=...
export MATRIX_USER=...
export MATRIX_PASSWORD=...

# Build
cargo build --release
```

---

## 2. Automated Test Suite

```bash
# Full workspace (all crates, ~1711 tests)
cargo test --workspace

# Individual crates
cargo test -p zeus-core
cargo test -p zeus-llm
cargo test -p zeus-agent
cargo test -p zeus-nous
cargo test -p zeus-prometheus
cargo test -p zeus-channels
cargo test -p zeus-talos
cargo test -p zeus-aegis
cargo test -p zeus-mnemosyne
cargo test -p zeus-tui

# Lint
cargo clippy --workspace -- -D warnings

# Format check
cargo fmt --check
```

Expected: all tests pass, 0 clippy warnings, 0 fmt warnings.

---

## 3. CLI Smoke Tests

```bash
# Config + diagnostics
zeus config
zeus doctor

# Single message (non-streaming)
zeus chat "Hello, what can you do?"

# Streaming
zeus chat -s "List the 8 core tools"

# Tool execution
zeus tool list_dir '{"path":"."}'
zeus tool read_file '{"path":"README.md"}'
zeus tool shell '{"command":"whoami"}'

# Memory
zeus memory show
zeus memory remember "test fact: Zeus is running"
zeus memory note "test daily note"

# Sessions
zeus session list
```

---

## 4. TUI

```bash
zeus tui   # or: zeus
```

### Screens to verify (Tab / number keys)

| Key | Screen | What to check |
|-----|--------|---------------|
| 1 | Chat | Type message, verify streaming response; tool calls render inline |
| 2 | Tools | List loads (193+ tools shown); search works |
| 3 | Memory | Workspace files listed; read workspace files |
| 4 | Agents | Agent profiles load from TOML |
| 5 | Status | Model/provider/session ID shown |
| 6 | Help | Keybindings render |
| 7 | Settings | Config fields editable |
| 8 | Teams | Agent teams listed |
| 9 | Extensions | Extensions list (may be empty) |
| 0 | Sandbox | Sandbox policies shown |

### TUI feature checklist

- [ ] Streaming response renders token-by-token
- [ ] Tool call result shows inline (e.g. `list_dir` output)
- [ ] Shift+Enter inserts newline, Enter sends
- [ ] `/clear` resets conversation
- [ ] Ctrl+C exits cleanly
- [ ] File upload preview (S15-T6): attach a file, verify extraction preview shown before send

---

## 5. API Server

```bash
zeus serve   # default port 3001
# or with port:
zeus serve -p 3001
```

### Core endpoints

```bash
BASE=http://localhost:3001

# Health
curl $BASE/health

# Status
curl $BASE/v1/status | jq

# Doctor
curl $BASE/v1/doctor | jq

# Stats
curl $BASE/v1/stats | jq

# Chat (non-streaming)
curl -s -X POST $BASE/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message":"Hello"}' | jq

# WebSocket streaming
# Use wscat or similar:
# wscat -c ws://localhost:3001/v1/ws

# Sessions
curl $BASE/v1/sessions | jq
curl -X POST $BASE/v1/sessions | jq

# Tools
curl $BASE/v1/tools | jq '.[].name'
# NOTE: POST /v1/tools/{name} requires {"arguments":{...}} wrapper — bare body returns 422
curl -X POST $BASE/v1/tools/list_dir \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"path":"."}}' | jq

# Memory
curl $BASE/v1/memory | jq
curl -X POST $BASE/v1/memory/remember \
  -H "Content-Type: application/json" \
  -d '{"fact":"API test fact"}' | jq

# Agents
curl $BASE/v1/agents | jq
curl -X POST $BASE/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name":"TestAgent","model":"anthropic/claude-haiku-4-5-20251001"}' | jq

# Channels (NOTE: use channel_type field, not type)
curl $BASE/v1/channels | jq
curl -X POST $BASE/v1/channels \
  -H "Content-Type: application/json" \
  -d '{"channel_type":"telegram","name":"My Telegram"}' | jq

# Personas (S20-3)
curl $BASE/v1/personas | jq
curl $BASE/v1/personas/code-reviewer | jq
curl -X POST $BASE/v1/agents/from-persona/tdd-guide \
  -H "Content-Type: application/json" \
  -d '{}' | jq

# Observer hook (S20-2)
curl -X POST $BASE/v1/nous/observe \
  -H "Content-Type: application/json" \
  -d '{"tool":"read_file","event":"tool_complete","success":true,"output":"file contents"}' | jq

# Approvals
curl $BASE/v1/approvals | jq

# Security
curl $BASE/v1/security/threats | jq
curl $BASE/v1/security/permissions | jq

# Analytics
curl $BASE/v1/analytics/costs | jq
curl $BASE/v1/analytics/tokens | jq
```

### OpenAI-compatible endpoint

```bash
curl -s $BASE/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"claude-haiku-4-5-20251001","messages":[{"role":"user","content":"ping"}],"stream":false}' | jq
```

---

## 6. Gateway (All Services)

```bash
zeus gateway
# Starts: API server + all channels + heartbeat + cron scheduler
```

Options:
```bash
zeus gateway --no-channels   # API only, no chat adapters
zeus gateway --no-cron       # skip cron jobs
zeus gateway --port 3001
```

Verify on startup log:
- `[prometheus] heartbeat started`
- `[cron] scheduler started`
- Channel adapters connected (one line per active channel)

---

## 7. Channels — Manual Testing

Each channel requires env vars configured. Test each adapter independently.

### 7.1 Telegram

**Env:** `TELEGRAM_API_ID`, `TELEGRAM_API_HASH`, `TELEGRAM_PHONE`

```bash
zeus gateway   # starts Telegram adapter via grammers-client MTProto
```

- Send a message to your Telegram account (or bot)
- Verify Zeus receives and replies
- Test: `list projects`, `create project TestProject` — intent parser should fire
- Test media: send an image or file — verify it's handled

### 7.2 Discord

**Env:** `DISCORD_BOT_TOKEN`, `DISCORD_RELAY_CHANNEL_IDS`

```bash
zeus gateway
```

- In the configured Discord channel, send `@Zeus ping`
- Verify bot replies
- Test relay: messages forwarded to relay channel
- Test: `zeus fleet status` command in fleet channel

### 7.3 Slack

**Env:** `SLACK_BOT_TOKEN` (xoxb-), `SLACK_APP_TOKEN` (xapp-), `SLACK_SIGNING_SECRET`

#### Setup (one-time)

1. **Create app** — [api.slack.com/apps](https://api.slack.com/apps) → Create New App → From scratch → name `Zeus` → pick your workspace

2. **Enable Socket Mode** — Settings → Socket Mode → Enable → generate App-Level Token with scope `connections:write` → copy the `xapp-...` token

3. **Bot Token Scopes** — OAuth & Permissions → Bot Token Scopes → add:
   `chat:write`, `channels:read`, `channels:history`, `groups:read`, `groups:history`, `im:read`, `im:history`, `reactions:write`, `files:write`

4. **Install to Workspace** — OAuth & Permissions → Install to Workspace → copy the `xoxb-...` Bot User OAuth Token

5. **Signing Secret** — Basic Information → App Credentials → copy Signing Secret

6. **Event Subscriptions** — Event Subscriptions → On → Subscribe to bot events: `message.channels`, `message.groups`, `message.im`

7. **Set credentials** (`~/.zeus/.env`):
   ```bash
   SLACK_BOT_TOKEN=xoxb-...
   SLACK_APP_TOKEN=xapp-...
   SLACK_SIGNING_SECRET=...
   ```

8. **Config** (`~/.zeus/config.toml`):
   ```toml
   [channels.slack]
   bot_token = ""    # reads from SLACK_BOT_TOKEN env var
   app_token = ""    # reads from SLACK_APP_TOKEN env var
   ```

9. **Invite bot to your channel** — in Slack: `/invite @Zeus`

10. **Get channel ID** — right-click channel → View channel details → copy ID at bottom (format: `C01234ABCDE`)

#### Testing

```bash
zeus gateway
# Watch for: [slack] connected via Socket Mode
```

```bash
BASE=http://192.168.1.226:3001

# List configured channels
curl $BASE/v1/channels | jq

# Send message — ⚠️ use channel ID (C01234ABCDE), NOT #channel-name
curl -X POST $BASE/v1/tools/slack_send_message \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"channel":"C01234ABCDE","text":"Hello from Zeus"}}' | jq

# Test connectivity
curl -X POST $BASE/v1/channels/slack/test | jq
```

- Mention `@Zeus` in the Slack channel → verify Zeus receives and replies
- Logs: `[slack] message received`, `[slack] reply sent`

### 7.4 Email (SMTP + IMAP)

**Env:** `SMTP_HOST`, `IMAP_HOST`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`

```bash
zeus gateway
```

- Send email to configured address
- Verify IMAP IDLE receives it (log: `[email] new message`)
- Verify reply sent via SMTP
- Check lettre delivery receipt

### 7.5 iMessage (macOS only)

**Requires:** macOS, Messages app open

```bash
zeus gateway   # starts iMessage AppleScript bridge
```

- Send iMessage to configured number
- Verify Zeus receives (log: `[imessage] message received`)
- Verify reply sent via AppleScript
- Note: `validate_recipient` confirms address format before send

### 7.6 WhatsApp

**Env:** `WHATSAPP_TOKEN`, `WHATSAPP_PHONE_NUMBER_ID`

> ⚠️ Env var is `WHATSAPP_TOKEN` — NOT `WHATSAPP_ACCESS_TOKEN` (old template name, now corrected)

#### Setup (one-time)

1. **Create Meta app** — [developers.facebook.com](https://developers.facebook.com) → My Apps → Create App → Business type

2. **Add WhatsApp product** — Dashboard → Add Product → WhatsApp → Set Up

3. **Get credentials** — WhatsApp → API Setup:
   - System Users → Generate token with `whatsapp_business_messaging` scope → copy token
   - Copy **Phone Number ID** from the dashboard (not the phone number itself)

4. **Add test recipient** (sandbox only) — add your personal WhatsApp number to the test recipients list. For production, complete Meta Business Verification.

5. **Set credentials** (`~/.zeus/.env`):
   ```bash
   WHATSAPP_TOKEN=EAAxxxxx...
   WHATSAPP_PHONE_NUMBER_ID=123456789012345
   ```

6. **Config** (`~/.zeus/config.toml`):
   ```toml
   [channels.whatsapp]
   mode = "cloud_api"
   ```

7. **Webhook for receiving** — set `https://YOUR_GATEWAY/v1/webhooks/whatsapp` as your Meta webhook URL (requires HTTPS — use ngrok or public IP)

#### Testing

```bash
zeus gateway
# Watch for: [whatsapp] adapter initialized
```

```bash
BASE=http://192.168.1.226:3001

# Send message — ⚠️ field is "to" (not "phone"), E.164 format (+1XXXXXXXXXX)
curl -X POST $BASE/v1/tools/whatsapp_send_message \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"to":"+1XXXXXXXXXX","message":"Hello from Zeus"}}' | jq

# Test connectivity
curl -X POST $BASE/v1/channels/whatsapp/test | jq
```

- Send a WhatsApp message to your registered number → Zeus receives via webhook → verify reply
- Logs: `[whatsapp] message received`, `[whatsapp] reply sent`

### 7.7 Signal

**Env:** `SIGNAL_PHONE`, `SIGNAL_CLI_PATH`

> ⚠️ Use `link` (secondary device) — NOT `register`. Registering deactivates Signal on your phone.

#### Setup (one-time)

1. **Install signal-cli**:
   ```bash
   # macOS
   brew install signal-cli   # requires Java 17+

   # FreeBSD (.226)
   sudo pkg install openjdk17
   # Download from github.com/AsamK/signal-cli/releases (latest tar.gz)
   tar xf signal-cli-0.13.x.tar.gz
   sudo cp signal-cli-0.13.x/bin/signal-cli /usr/local/bin/
   ```

2. **Link as secondary device** (keeps Signal working on your phone):
   ```bash
   signal-cli link --name "Zeus-226"
   # Prints a tsdevice:/ URI — paste into a QR generator or use:
   # qrencode -t ANSI "tsdevice:/?..."
   # Then in Signal app → Settings → Linked Devices → Link a Device → scan QR
   ```

3. **Verify link**:
   ```bash
   signal-cli -u +1YOURPHONE send -m "test" +1YOURPHONE
   # Should receive the message on your phone
   ```

4. **Set credentials** (`~/.zeus/.env`):
   ```bash
   SIGNAL_PHONE=+1YOURPHONE
   SIGNAL_CLI_PATH=/usr/local/bin/signal-cli
   ```

5. **Config** (`~/.zeus/config.toml`):
   ```toml
   [channels.signal]
   phone = "+1YOURPHONE"
   signal_cli_path = "/usr/local/bin/signal-cli"
   ```

#### Testing

```bash
zeus gateway
# Watch for: [signal] JSON-RPC subprocess started
```

```bash
BASE=http://192.168.1.226:3001

# Send message
curl -X POST $BASE/v1/tools/signal_send_message \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"phone":"+1RECIPIENTNUMBER","message":"Hello from Zeus"}}' | jq

# Test connectivity
curl -X POST $BASE/v1/channels/signal/test | jq
```

- Send a Signal message to your registered number → verify Zeus receives and replies
- Logs: `[signal] message received`, `[signal] reply sent`

### 7.8 Matrix

**Env:** `MATRIX_HOMESERVER`, `MATRIX_USER`, `MATRIX_PASSWORD`

```bash
zeus gateway
```

- Join a Matrix room
- Send message to Zeus Matrix user
- Verify matrix-sdk v0.16 receives (log: `[matrix] room message`)
- Verify reply in room

---

## 8. Cognitive Engine (zeus-nous)

```bash
cargo test -p zeus-nous
```

Key test coverage:
- Intent recognition: `test_intent_parse_*`
- Confidence decay + persist (S19-3): `test_apply_decay_all_persisted`, `test_update_lesson_persisted`
- Cold reload survival: lesson confidence survives process restart via Mnemosyne SQLite
- Autonomy decisions: `test_autonomy_*`
- Learning from outcomes: `test_learning_*`

Manual verification:
```bash
zeus chat "Remember that I prefer short responses"
zeus chat "What do you know about my preferences?"  # should recall
```

---

## 9. Memory (zeus-mnemosyne)

```bash
cargo test -p zeus-mnemosyne
```

Features:
- SQLite FTS5 full-text search
- Vector embeddings (requires Ollama `nomic-embed-text` or OpenAI)
- Hybrid search: BM25 + cosine similarity

```bash
# Verify search
curl -X POST http://localhost:3001/v1/memory/search \
  -H "Content-Type: application/json" \
  -d '{"query":"test fact","mode":"hybrid"}' | jq
```

---

## 10. Scheduler / Cron (zeus-prometheus)

```bash
cargo test -p zeus-prometheus
```

Key tests:
- `drain_empty_queue_returns_ok` — ContentQueueDrain with empty queue
- `drain_bad_path_returns_false` — bad DB path returns (false, err)
- `drain_serializes_correctly` — TaskType::ContentQueueDrain roundtrips via serde

```bash
# Schedule a content queue drain job via API
curl -X POST http://localhost:3001/v1/tools/shell \
  -H "Content-Type: application/json" \
  -d '{"command":"ls ~/.zeus/"}' | jq
```

---

## 11. Security (zeus-aegis)

```bash
cargo test -p zeus-aegis   # macOS: includes Seatbelt tests
```

Features to verify:
- Command filtering: `zeus tool shell '{"command":"rm -rf /"}'` → should be denied
- URL allowlist: `zeus tool web_fetch '{"url":"http://malicious.example"}'` → denied if not allowlisted
- Approvals API: sensitive ops create pending approval

```bash
curl http://localhost:3001/v1/security/threats | jq
curl http://localhost:3001/v1/security/allowlist | jq
```

---

## 12. Talos macOS Automation (193 tools)

**Requires:** macOS

```bash
cargo test -p zeus-talos
```

Sample tool tests:
```bash
zeus tool system_info '{}'
zeus tool screenshot '{}'
zeus tool clipboard_read '{}'
zeus tool spotlight_search '{"query":"Zeus"}'
zeus tool process_list '{}'
zeus tool wifi_list '{}'
```

---

## 13. Pantheon (Multi-agent)

```bash
# Create a mission
curl -X POST http://localhost:3001/v1/missions \
  -H "Content-Type: application/json" \
  -d '{"title":"Test mission","goal":"List all available tools"}' | jq

# Check Pantheon economy stats (S19-1 fix)
curl http://localhost:3001/v1/pantheon/economy | jq
# Expected: { "stats": {6 fields}, "agents": [{7 fields}] }
# NOT: { "wallets": ..., "transactions": ... }
```

---

## 14. Native Apps

### macOS Desktop (ZeusDesktop)

```bash
open apps/Zeus.xcworkspace
# Build + run in Xcode
```

- Verify 3-column layout: Dashboard / Chat / Tools
- Chat: send message, verify streaming
- Tools: browse tool list
- Memory: workspace files accessible
- Settings: config editor

### iOS (ZeusMobile)

- Build via Xcode targeting iOS simulator or device
- TabView: Home / Sessions / Chat / Tools / Memory / Settings
- Test chat with gateway running at configured IP

### Web (ZeusWeb — Leptos/WASM)

```bash
cd apps/ZeusWeb
trunk serve   # dev server
```

Visit http://localhost:8080 — verify:
- Dashboard loads
- Chat page (session list + message thread)
- Sessions page
- Tools browser
- Memory viewer
- Channels status
- Analytics
- Security
- Pantheon page (reactions, reply, edit, delete — S16)

### visionOS (zeus-vision)

```bash
# Separate repo: ~/zeus-vision/
open ~/zeus-vision/ZeusVision.xcodeproj
# Target: Apple Vision Pro simulator
```

---

## 15. Fleet / Multi-agent

```bash
zeus fleet list
zeus fleet status
zeus fleet add --name agent1 --ip 192.168.1.107 --port 3001
```

API:
```bash
curl http://localhost:3001/v1/fleet/status | jq
curl http://localhost:3001/v1/network/agents | jq
```

---

## 16. Daemon

```bash
zeus daemon install    # installs launchd plist
zeus daemon start
zeus daemon status
zeus daemon stop
```

Verify:
- `launchctl list | grep zeus`
- Logs at `~/.zeus/zeus.log`
- Gateway auto-restarts on crash

---

## 17. OpenAI-Compatible Mode

Zeus exposes a drop-in OpenAI API. Test with any OpenAI client:

```python
import openai
client = openai.OpenAI(base_url="http://localhost:3001/v1", api_key="any")
response = client.chat.completions.create(
    model="claude-haiku-4-5-20251001",
    messages=[{"role": "user", "content": "ping"}]
)
print(response.choices[0].message.content)
```

---

## 18. Quick Regression Checklist (Post-merge)

Run after any main merge:

```bash
cargo test --workspace                       # all tests pass
cargo clippy --workspace -- -D warnings      # 0 warnings
cargo fmt --check                            # 0 fmt issues
zeus doctor                                  # all checks green
zeus chat "What tools do you have?"         # agent loop works
zeus tool list_dir '{"path":"."}'           # tool execution works
curl http://localhost:3001/v1/status | jq   # API up
```

---

## Known Environment-Gated Tests (S19-4)

Some tests only run when specific env vars are set:

| Env var | Tests gated | Note |
|---------|-------------|------|
| `ZEUS_HAS_LLM=1` | 3 LLM integration tests | Requires real API key |
| `ZEUS_HAS_WHATSAPP=1` | 1 WhatsApp integration test | Requires Cloud API token |

Without these vars, the tests are skipped (not failed) — this is correct behavior.

---

## 19. FreeBSD Notes (.224 / .226)

**Prerequisites**
```sh
pkg install rust llvm openssl sqlite3 ffmpeg node npm
ulimit -n 10240   # required before cargo build or gateway start
```

**Env vars** — `ANTHROPIC_API_KEY` must be `export`ed in the shell before launching TUI or gateway. The TUI does NOT auto-read `.env` files.

**ZeusWeb** — pin wasm-bindgen-cli to the exact version in `apps/ZeusWeb/Cargo.lock` before running `trunk serve`. See `scripts/deploy-freebsd.sh` for the correct install command.

**Test suite** — 2 pre-existing config-dependent failures on .226 (read live `~/.zeus/` state) — non-blocking, pre-existing. Use `tempdir` workspace pattern for hermetic tests.

**TTS** — Piper TTS binary not in FreeBSD pkg; zeus-tts falls back to system TTS gracefully.
