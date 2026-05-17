# Zeus MCP Tester Guide — TUI · macOS Desktop · Web

> For: MCP-based testers using Zeus MCP shell tools
> Main: `02909a29` · Date: 2026-03-02

---

## Setup (do once)

```
mcp__zeus__shell: cd /Users/mike/Zeus && git pull origin main
```

Confirm you're on `02909a29`:
```
mcp__zeus__shell: cd /Users/mike/Zeus && git rev-parse HEAD
```

Make sure the gateway is running (needed for TUI + Web API calls):
```
mcp__zeus__shell: pgrep -x zeus || echo "NOT RUNNING"
```

If not running:
```
mcp__zeus__shell: cd /Users/mike/Zeus && nohup cargo run --release -- gateway > /tmp/zeus-gateway.log 2>&1 &
```

---

## 1. TUI Testing

### Launch
```
mcp__zeus__shell: cargo run --release -- tui
```

Or if already built:
```
mcp__zeus__shell: ~/.local/bin/zeus tui
```

### Screens checklist — Tab or number keys (1–9, 0)

| Key | Screen | What to verify |
|-----|--------|----------------|
| 1 | **Chat** | Type a message, verify streaming response comes back. Try: `"list your tools"` |
| 2 | **Tools** | Tool list loads (should show 193+ tools). Try searching with `/` |
| 3 | **Memory** | Workspace files listed. Try navigating to `MEMORY.md` |
| 4 | **Agents** | Agent profiles listed (may be empty if none configured) |
| 5 | **Status** | Shows model name, provider, session ID, workspace path |
| 6 | **Help** | Keybindings table renders correctly |
| 7 | **Settings** | Config fields visible and editable |
| 8 | **Teams** | Teams screen loads (may be empty) |
| 9 | **Extensions** | Extensions list loads |
| 0 | **Sandbox** | Sandbox policies visible |

### Key interactions to verify
- **Send message:** Type text, press Enter → response streams token-by-token
- **Newline in input:** Shift+Enter inserts newline without sending
- **Tool call render:** Ask `"list files in ."` → tool result should appear inline
- **File upload preview (S15-T6):** Attach a file in the chat input — extraction preview should appear before send
- **Clear session:** Type `/clear` → conversation resets
- **Exit cleanly:** Ctrl+C → TUI exits without hang or panic

### Quick functional test via MCP
```
mcp__zeus__shell: echo '{"message":"ping"}' | zeus chat
```
Expected: response text returned, no crash.

---

## 2. macOS Desktop (ZeusDesktop)

### Build via Xcode
```
mcp__zeus__shell: open /Users/mike/Zeus/apps/Zeus.xcworkspace
```
Then in Xcode: select **ZeusDesktop** scheme → **My Mac** target → **Cmd+R** to build and run.

Or build from command line:
```
mcp__zeus__shell: xcodebuild -workspace /Users/mike/Zeus/apps/Zeus.xcworkspace -scheme ZeusDesktop -destination "platform=macOS" build 2>&1 | tail -20
```
Expected last line: `** BUILD SUCCEEDED **`

### UI checklist (3-column layout)

**Left sidebar — navigation:**
- [ ] Dashboard item visible and selectable
- [ ] Chat item visible and selectable
- [ ] Tools item visible and selectable
- [ ] Memory item visible and selectable
- [ ] Settings item visible and selectable

**Dashboard:**
- [ ] Status overview loads (model, session count, memory stats)
- [ ] No blank screen or crash on load

**Chat:**
- [ ] Send a message → streaming response renders
- [ ] Tool calls render inline in chat bubbles
- [ ] Session list in sidebar shows previous sessions

**Tools:**
- [ ] Tool browser shows 193+ tools
- [ ] Tool search filters results
- [ ] Execute a tool (e.g. `list_dir`) from the browser

**Memory:**
- [ ] Workspace files listed
- [ ] Can read/display a file (e.g. `MEMORY.md`)

**Settings:**
- [ ] Config fields populated (model, workspace path)
- [ ] Editable without crash

**Menu bar:**
- [ ] Zeus menu bar icon visible when app running
- [ ] Clicking icon shows quick-access panel

### Economy screen (S19-1 fix — critical)
Navigate to the Economy/Agora screen:
- [ ] Stats section shows 6 fields: `total_listings`, `active_listings`, `total_trades`, `completed_trades`, `total_token_supply`, `total_agents`
- [ ] Agents list shows entries with 7 fields: `agent_id`, `balance`, `trust_score`, `trades_completed`, `trades_failed`, `badge`, `badge_color`
- [ ] **NOT** showing `wallets` or `transactions` keys (those were the S19-1 bug)

---

## 3. Web (ZeusWeb — Leptos/WASM)

### Launch dev server
```
mcp__zeus__shell: cd /Users/mike/Zeus/apps/ZeusWeb && trunk serve
```

Wait for: `server listening at http://localhost:8080`

Or check if already running:
```
mcp__zeus__shell: curl -s -o /dev/null -w "%{http_code}" http://localhost:8080
```
Expected: `200`

### Pages checklist

Visit each URL and verify it loads without blank screen / console errors:

| URL | Page | What to check |
|-----|------|---------------|
| `/` | Dashboard | Stats widgets load, no blank boxes |
| `/chat` | Chat | Session sidebar + message thread visible |
| `/sessions` | Sessions | Session list loads with metadata |
| `/tools` | Tools | Tool browser shows 193+ tools, search works |
| `/memory` | Memory | Workspace file tree visible |
| `/memory/files` | Memory files | File list with read capability |
| `/channels` | Channels | Channel status indicators (connected/disconnected) |
| `/analytics/costs` | Analytics | Cost breakdown chart or table |
| `/analytics/tokens` | Analytics | Token usage chart |
| `/security/threats` | Security | Threat log (may be empty, that's OK) |
| `/security/permissions` | Security | Permission matrix visible |
| `/agents` | Agents | Agent list (may be empty) |
| `/agents/spawn` | Spawn | Spawn form renders |
| `/projects` | Projects | Project list |
| `/pantheon` | Pantheon | Rooms/missions visible |
| `/pantheon/economy` | Economy (S19-1) | See Economy check below |
| `/skills` | Skills | Skill browser loads |
| `/settings` | Settings | Config editor renders |
| `/status` | Status | Server status, model, version |
| `/doctor` | Doctor | Diagnostic checks run and show results |

### Economy page (S19-1 fix — critical)
Navigate to `/pantheon/economy`:
- [ ] Stats section shows: total_listings · active_listings · total_trades · completed_trades · total_token_supply · total_agents
- [ ] Agent rows show: agent_id · balance · trust_score · trades_completed · trades_failed · badge
- [ ] No `wallets` or `transactions` section (that was the bug)

### WebSocket streaming
On the `/chat` page:
- [ ] Send a message → response streams in real-time (not a full-page reload)
- [ ] Pantheon War Room: join a room → messages update live via WS

### API smoke test (from MCP)
```
mcp__zeus__shell: curl -s http://localhost:3001/v1/status | python3 -m json.tool
mcp__zeus__shell: curl -s http://localhost:3001/v1/tools | python3 -c "import sys,json; t=json.load(sys.stdin); print(f'{len(t)} tools')"
```
Expected: status object + `193 tools` (or similar)

---

## 4. Quick Regression (run after any fix)

```
mcp__zeus__shell: cd /Users/mike/Zeus && cargo test --workspace 2>&1 | tail -5
mcp__zeus__shell: cd /Users/mike/Zeus && cargo clippy --workspace -- -D warnings 2>&1 | tail -5
```

Expected: all tests pass, 0 clippy warnings.

---

## 5. Reporting Issues

Post findings to Discord #private with:
- **Frontend**: TUI / macOS Desktop / Web
- **Screen/page**: exact screen name or URL
- **What happened**: actual behavior
- **What expected**: correct behavior
- **Repro**: steps to reproduce

Tag `@zeus106` for Rust/API issues, `@Zeus100` for Apple app issues.
