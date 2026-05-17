# PRD — Production TUI (post-onboarding)

**Date:** 2026-05-03
**Author:** Zeus100
**Status:** Draft for merakizzz design pass
**Scope:** TUI experience after onboarding completes — the day-to-day production interface in `crates/zeus-tui/`. NOT the onboarding wizard (separate PRD: `prd-tui-onboarding-comprehensive-2026-05-03.md`).
**Trigger:** merakizzz directive 2026-05-03 — *"give me a PRD for our TUI as it stands now, not onboarding but production tui."*
**Source of truth for current state:** `crates/zeus-tui/src/app.rs:1305` (`Tab` enum), `crates/zeus-tui/src/ui.rs:148-164` (tab bar render), `crates/zeus-tui/src/screens/`, `crates/zeus-tui/src/office/`, `crates/zeus-tui/src/pantheon/`.

---

## Current state (verified against code, 2026-05-03)

### Tab bar — 4 primary tabs

```
zeus-titan • connected MiniMax-M2.7 • ctx [▓▓░░░░░░░░] 0% • Ctrl+C quit
 chat │ ⚡ the office │ pantheon │ settings  Tab to switch
```

**Tabs:**
1. **chat** (default) — main conversation surface
2. **⚡ the office** — pixel-art Phaser-like canvas, agents roam between zones (Engineering/Comms/Research/Break Room/Kitchen)
3. **pantheon** — multi-agent war rooms + missions (currently a stub: `pantheon/mod.rs` is 24 lines)
4. **settings** — configuration editor

**Sub-screens** (not in tab bar, accessed via keybinds or from settings):
- `screens/help.rs` — keybinds reference
- `screens/memory.rs` — workspace files browser
- `screens/status.rs` — model/session/path info

**Note on CLAUDE.md staleness:** CLAUDE.md claims 23 screens with `TAB_COUNT = 23`. Actual code has 4 tabs. The S61 "8 primary + Advanced" structure was further simplified to today's 4. The PRD documents the actual current state, not the historical claim.

---

## Goals + UX principles

1. **Surface every backend feature.** Per merakizzz directive 2026-05-03 — *"literally all features we already have"* — the TUI must expose every capability that exists in the backend (tools, memory, agents, channels, analytics, security, approvals, skills, MCP, projects, canvas, voice, vectorstores, economy, etc.). The current 4-tab structure is a REGRESSION from the historical 23-screen design and must be reversed. New target: 8 primary tabs + Advanced submenu (per S61 pattern), all features accessible via keybinds or tab navigation.
2. **Chat is the home tab,** not the only tab. Operators do work in chat but should reach any subsystem in 1-2 keystrokes (Tab cycle or `Ctrl+K` palette).
3. **Always-visible status.** Top status bar shows connection, model, context-window utilization, and quit hint. Bottom bar shows context-sensitive keybinds.
4. **Streaming feels alive.** Streaming responses, thinking indicators, cooking-loop progress, and live tool-call stream — all visible without leaving chat.
5. **No modals if avoidable.** Settings is a tab, not a popup. Help is `?` overlay. Confirmations use inline yes/no rather than modal blocks.
6. **Operator hand-off ready.** Connection state, error states, and pending approvals are visible at a glance — never silent failures.
7. **Match the WebUI/native apps stylistically** where it makes sense (Orbitron-equivalent monospace headers, fire-orange accent, dark theme), but respect terminal constraints (no Unicode-heavy graphics that fall back ugly on Apple Terminal).

---

## Feature inventory — every backend capability must have a TUI surface

Per merakizzz: *"literally all features we already have"* must be reachable from the TUI. The current 4-tab structure (chat / office / pantheon / settings) covers <20% of the platform's capability surface. Everything below either needs a dedicated tab, an Advanced-submenu entry, or a sub-view from an existing tab.

| # | Feature | Backend crate | Current TUI surface | Required TUI surface |
|---|---------|---------------|---------------------|----------------------|
| 1 | **Chat** (conversational core) | `zeus-agent` | ✅ `chat` tab | (current) |
| 2 | **Office** (presence + activity) | `zeus-orchestra` | ✅ `office` tab | (current, post Track B) |
| 3 | **Pantheon** (war rooms + missions) | `zeus-orchestra` + `zeus-prometheus/mission_driver` | 🟡 24-line stub | Mission list, war room chat, plan card approval, live event feed |
| 4 | **Settings** | `zeus-core/Config` | ✅ flat 17-field list | 2-pane subsystem grouping |
| 5 | **Tools browser** | `zeus-talos` + `zeus-mcp` + `zeus-agent/tools` | ❌ none | Browse all tools by category, search, see schemas, execute on demand |
| 6 | **Memory** (workspace + sessions) | `zeus-memory` + `zeus-mnemosyne` + `zeus-session` | ⚠️ `screens/memory.rs` exists but not a tab | Tree view of `~/.zeus/workspace/`, session list, JSONL replay, Mnemosyne FTS+vector search |
| 7 | **Agents** (profiles, fleet) | `zeus-agent` + `zeus-orchestra` | ❌ none | List agents (local + fleet), view personas, edit bindings, send message to agent, see status |
| 8 | **Channels** | `zeus-channels` (8 adapters) | ⚠️ visible in status as channels list, not editable | List channels with status, send test message, edit credentials, view recent messages, pause/resume |
| 9 | **Approvals** (Aegis pending tool exec) | `zeus-aegis` | ❌ none | List pending approvals, approve/deny inline, view tool args + reason for sandbox flag |
| 10 | **Sandbox / Security** (Aegis) | `zeus-aegis` | ❌ none | View sandbox level, command allowlist, URL allowlist, threat log, audit trail |
| 11 | **Analytics** (tokens + cost) | `zeus-api/routes/analytics` | ❌ none | Token usage breakdown, per-provider costs, budget thresholds, session-level cost |
| 12 | **Skills** | `zeus-skills` | ❌ none | List installed skills, browse marketplace, install/uninstall, enable/disable, view SKILL.md |
| 13 | **MCP servers** | `zeus-mcp` | ❌ none | List connected servers, view their tools, connect/disconnect, server health |
| 14 | **Projects** | (stub) | ❌ none | Create/list projects, assign agents, view project status |
| 15 | **Canvas** (visual plan/workflow builder) | (stub) | ❌ none | Build/run plans visually, drag-drop tool nodes (terminal-friendly: ASCII tree) |
| 16 | **Voice** (calls + STT/TTS) | `zeus-voice` + `zeus-tts` | ❌ none | Start/end voice calls, STT/TTS configuration, voice presets, recording history |
| 17 | **NodeComms** (fleet messaging) | `zeus-orchestra` | ❌ none | Discord-channel-equivalent within the TUI for inter-agent messages |
| 18 | **VectorStores** (browse Mnemosyne) | `zeus-mnemosyne` | ❌ none | List vector stores, query semantic search, view embeddings, manage collections |
| 19 | **Economy** (Agora wallet + marketplace) | `zeus-economy` + `zeus-agora` + `zeus-wallet` | ❌ none | Wallet balance, transactions, marketplace listings, skill purchases, x402 settlement |
| 20 | **Extensions** | `zeus-extensions` | ❌ none | List installed extensions, install Deno/MCP extensions, runtime status |
| 21 | **Knowledge graph** | `zeus-mnemosyne/graph` | ❌ none | Browse memory graph, communities, relationships, traversal queries |
| 22 | **Spawner** (subagent runtime) | `zeus-agent/subagent` | ⚠️ `/spawn` slash cmd only | List active subagents, kill, view their conversation logs |
| 23 | **Deploy / Daemon status** | (CLI `zeus daemon`) | ❌ none | Daemon health, restart, view launchd logs (when fixed via `5e6dc4f6`), service install/remove |

**Net:** 4 features have proper TUI surfaces today (chat, office, pantheon-stub, settings). **19 features have no TUI access** despite having full backend implementations + (in many cases) WebUI parity. The TUI is currently the OPERATOR PRIMARY INTERFACE for fleet hosts — this gap is severe.

### Proposed restructure: 8 primary tabs + Advanced submenu

Mirroring the S61 pattern that got reverted/lost. Primary tabs = highest-frequency operations:

1. **chat** — conversational core (current)
2. **office** — presence + activity (current)
3. **pantheon** — war rooms + missions (needs implementation)
4. **tools** — tool browser + executor (NEW)
5. **memory** — workspace + sessions + Mnemosyne (NEW)
6. **channels** — adapter management (NEW)
7. **approvals** — pending tool exec + Aegis (NEW)
8. **settings** — config editor (current, restructured)

**Advanced submenu** (accessed via `:` or a 9th "more" tab): agents, skills, MCP, projects, canvas, voice, nodecomms, vectorstores, economy, extensions, knowledge-graph, spawner, deploy. Mirrors WebUI's "Advanced" sidebar — keeps primary tab bar focused while still exposing every feature.

---

## Per-tab PRDs

### Tab 1 — `chat` (default, primary work surface)

**File:** `crates/zeus-tui/src/screens/chat.rs` (306 lines).

**Current state:**
- Message history pane (scrollable, role-coloured: user/assistant/tool)
- Input field at bottom with cursor + submit on Enter
- Streaming responses render incrementally with markdown styling
- Cooking-loop iteration counter (`cooking_iter` / `cooking_tools` / `total_tools`)
- Thinking indicator when LLM is mid-response (`thinking_text`)
- Channel-source badge if message came from Discord/Telegram/etc. (`channel_source`)

**Pain points / regressions surfaced today:**
- No visible model name in chat itself (only in top status bar) — operators forget which provider they're using mid-conversation
- Tool-call expansion uses 15-line auto-fold (per S56) but no clear "click here for full output" affordance
- Streaming thinking-text occasionally gets stuck after stream end (banked S60 issue, may still be live)

**Design improvements:**
- (a) **Per-message provider badge** — tiny tag next to assistant messages showing which provider/model produced this turn. Helpful for fallback-chain debug.
- (b) **Clearer tool-call expansion** — current auto-fold is opaque. Add `[+]` / `[-]` glyph + a visible "12 more lines" hint, and a keybind (e.g., `e`) to expand the focused message.
- (c) **Persistent context-window indicator** — the top-bar `ctx [▓▓...]` is good but small. Surface a more visible warning when context > 80% full ("Approaching context limit — consider /compact").
- (d) **Slash command surface** — currently slash commands (`/help`, `/clear`, `/compact`, etc.) are hidden. Add a `/` overlay that shows available commands with descriptions as the user types.
- (e) **Channel-message visual distinction** — incoming messages from Discord/Slack/Telegram should be visually grouped or threaded so the operator can distinguish channel traffic from direct chat.
- (f) **Message queuing (mandatory — merakizzz directive 2026-05-03):** the input field must NEVER block while the agent is processing. Operators type the next message during streaming/cooking-loop iteration; messages stack into a FIFO queue and fire one-by-one as each turn completes. See dedicated section below.

---

### Tab 2 — `⚡ the office` (pixel-art presence + activity)

**File:** `crates/zeus-tui/src/office/mod.rs` (680 lines, just fixed today via `93d88623`).

**Current state (post today's fixes):**
- Pixel-art canvas filling left ~75% of terminal area (after self-heal fix)
- 5 zones rendered: Engineering, Comms, Research, Break Room, Kitchen
- Right-side sidebar (26 cols): AGENTS list (local + channel) / ZONES counts / STATS (Ticks/Active/Idle/Errors)
- Animated 8 TPS tick (agents tick + connection status update)
- Bottom info bar: tab list + "Tab to switch"

**Pain points / regressions surfaced today:**
- ⚠️ **TUI Track B still in flight (zeus-spark):** AGENTS sidebar shows "1 local, 1 channel" when fleet has 7+ Discord-channel agents — same field-name mismatch as the WebUI bug we fixed. Will land soon.
- The "Activity" counters (Ticks 0, Active 0, Idle 0) currently stay at 0 even when agents are visibly walking around — disconnect between the Phaser-like animation tick and the reported counters.
- No way to focus on a specific agent and see what they're doing — clicking/keybind-selecting a sprite should pop a side panel.

**Design improvements:**
- (a) **Agent-focus panel.** Press `f` (already wired for "cycle focus") to highlight the next agent; selected agent's name + status + current task should appear in a small overlay or in the sidebar's selected-agent slot.
- (b) **Speech bubbles for active agents.** Per S62 spec — agents emitting messages should briefly render speech bubbles above their sprites. Confirm this works post-Track-B-merge.
- (c) **Trust glow.** Per S62 spec — agents with high reputation/trust should have a subtle aura. Verify implementation status.
- (d) **Zone affordances.** Each zone (Engineering/Comms/Research/Break Room/Kitchen) should map to an actual operational concept (e.g., Engineering = currently coding; Research = browsing/searching; Break Room = idle). Currently zones are decorative.
- (e) **Reconcile counters with sprites.** Either fix the counter wiring so Active/Idle count matches the visible sprites, OR remove the counters and let the visuals speak.

---

### Tab 3 — `pantheon` (multi-agent war rooms — STUB)

**File:** `crates/zeus-tui/src/pantheon/mod.rs` (only 24 lines — placeholder).

**Current state:** Effectively unimplemented in the TUI. The Pantheon backend exists (35 API routes, war rooms, missions, plan cards — per CLAUDE.md), and the WebUI has a full Pantheon page, but the TUI's pantheon tab is a stub that just renders a "coming soon" message (or similar minimal content).

**Pain points:**
- The TUI is the operator's primary interface but the Pantheon multi-agent feature (which is one of the platform's key differentiators) has no first-class TUI surface.
- Operators can't review or approve Plan Cards from the TUI — must use WebUI.

**Design improvements (large scope — likely post-launch sprint):**
- (a) **Mission list pane** — list active missions with status (Draft/Planning/Assembling/Active/Reviewing/Completed). Keybinds: `n` new mission, `p` pause, `c` cancel, `r` redirect.
- (b) **War room chat pane** — pick a room from a list, see real-time message stream. SSE consumption from `/v1/pantheon/rooms/:id/stream`.
- (c) **Plan card approval inline** — when a mission generates a plan card, the TUI should surface it in the pantheon tab with `a` approve / `r` reject keybinds.
- (d) **Live event feed** — running missions emit events; the tab should show a real-time feed similar to the office's activity counters but mission-scoped.

For launch tomorrow: keep the stub as-is. Pantheon TUI is post-launch work.

---

### Tab 4 — `settings` (configuration editor)

**File:** `crates/zeus-tui/src/screens/settings.rs` (139 lines) + `settings_fields.rs`.

**Current state:**
- 17 settings entries (per the test handler that caps `settings_cursor < 15` plus 2 — fields are model, provider, gateway URL, theme, vim_mode, max_iterations, channel toggles, advanced flags, etc.)
- ↑/↓ navigation, Enter to edit, Esc to back to chat
- Inline edit mode: shows current value, captures key input

**Pain points:**
- Flat list of 17 entries — no grouping. Operators scroll past channel settings to find a memory toggle.
- No visible save/discard semantics — when does an edit persist? Is it on Enter? On Esc?
- Some advanced fields (e.g., Aegis sandbox level, Mnemosyne db path) require knowing what they do — no inline help.

**Design improvements:**
- (a) **Group by subsystem** — Settings should have a 2-pane layout: left = subsystem list (LLM / Channels / Memory / Security / Tools / Display), right = the fields for the selected subsystem. Mirrors the way the WebUI groups settings.
- (b) **Visible save indicator** — when a field is dirty (changed but not saved), show a `*` next to it. After save, show a brief flash confirmation.
- (c) **Inline help (`?`)** — pressing `?` on a focused setting shows a one-line description in a footer area.
- (d) **Validation** — invalid values (port out of range, malformed URL) should reject the edit with a clear error inline rather than silently accepting and breaking on next gateway start.
- (e) **Re-entry to onboarding** — add a "Re-run onboarding wizard" entry at the top of settings. Currently the only way is to delete `~/.zeus/config.toml` + relaunch.

---

## Mandatory chat-tab behaviors (merakizzz directives 2026-05-03)

### Message queuing — input never blocks

**Directive:** *"tui must allow message queuing, etc"*

**Required behavior:**

- **Input field is always live.** While the agent is mid-stream, mid-cooking-loop, or mid-tool-call, the input field accepts keystrokes immediately. No locked/disabled state.
- **FIFO queue.** Pressing Enter while a turn is in progress pushes the message onto a queue (visible to operator), not into a discarded void. Each turn completion pops the next queued message and fires it.
- **Visible queue indicator.** Above the input bar, render a 1-line queue summary when items are pending:
  ```
  [Queued: 3 messages — Esc to cancel last, Ctrl+Esc to clear all]
  ```
- **Queue management keybinds:**
  - `Esc` (when nothing in input field) — cancel the last queued message
  - `Ctrl+Esc` — clear all queued messages
  - `↑` (when input empty) — pull the most recent queued message back into the input for editing
- **No interrupt of in-flight turn** — queued messages WAIT until current turn completes naturally. To interrupt the current turn, use existing `Ctrl+C` 3-strike (S77 pattern).
- **Cross-channel applies too** — incoming Discord/Telegram messages while local user is queuing must also queue, not interleave mid-stream. Channel-source messages render with their badge in the queue display.

**Why critical:** the current behavior (input blocks during streaming) makes the TUI feel sluggish for power users who want to dictate a multi-step plan upfront and walk away. Queuing turns the TUI into a true async work surface.

**Implementation pointers:**
- `App` already has `input: String` + `messages: Vec<ChatMessage>`. Add `pending_queue: VecDeque<QueuedMessage>`.
- The cooking-loop completion signal in `prom_guard.cook()` is the trigger to pop from queue. Hook there.
- Render queue indicator in the layout slot just above the input field — currently empty space when no streaming activity.

---

### Live tool usage display

**Directive (merakizzz 2026-05-03):** *"also show tool usage from the agent"*

**Current state:** TUI shows aggregate counters only (`cooking_iter` / `cooking_tools` / `total_tools`). Operators see "iter 3 of 8, 12 tools used" but can't tell WHICH tools or with what args.

**Required behavior:**

- **Live tool-call stream in chat history.** When the agent invokes a tool, render an inline pseudo-message in the chat scroll:
  ```
  ⚙ tool_call · read_file({"path":"/etc/hosts"})
    ↳ 24 lines returned (expand: e)
  ⚙ tool_call · shell({"command":"git status"})
    ↳ running...
  ```
- **Real-time status.** Tool calls move through three states: `running...` (spinner) → `success` (✓) / `failed` (✗) → final output (truncated to 5 lines, expand to see all).
- **Argument visibility.** Show the tool args (truncated if large) inline. Operators can audit what the agent is actually doing without leaving chat.
- **Output expansion.** Press `e` while a tool-call message is focused (with arrow keys) to expand its full output. Default truncation: 5 lines + "X more lines".
- **Failed tool calls highlighted.** Red `✗` glyph + the error message (full, not truncated). Critical for debugging when the agent loops on a broken tool.
- **Tool aggregate stays.** The existing `cooking_tools` counter in the status bar remains as a quick-glance summary, but the chat scroll now shows the detail.

**Why critical:** without per-call visibility, operators can't see WHY a turn is taking long, WHICH tool failed, or WHAT data was passed. This is core to "operator-controlled tool use" — you can't control what you can't see.

**Implementation pointers:**
- The agent loop already emits tool-call events through the prom_guard cooking pipeline. Need a hook that pushes them into `App.messages` as a new role variant (e.g., `Role::ToolCall { name, args, status, output }`).
- Render path in `chat.rs` adds a new branch for the ToolCall role with the formatting above.
- Aegis approval flow (when sandbox blocks a call) should also surface here as a fourth state: `awaiting approval`.

---

## Cross-cutting design

### Status bar (top, 1-line)

**Current:** `<host> • <connection> <model> • ctx [<bar>] <%> • <quit-hint>`

**Improvements:**
- Add a per-tab indicator (e.g., a small badge showing pending approvals count when you're not on the approvals view, but that view doesn't exist yet)
- Color-code connection state: green=connected, yellow=reconnecting, red=disconnected
- Surface gateway version next to model — useful for fleet debugging when versions drift across hosts

### Tab bar (just below status)

**Current:** `chat │ ⚡ the office │ pantheon │ settings  Tab to switch`

**Improvements:**
- Add unread indicators on tabs (e.g., chat shows `(3)` if 3 new messages while user was on another tab)
- Pantheon tab badge if pending plan cards await approval
- Remove the `⚡` from "the office" — inconsistent (only one tab has an emoji); either all tabs get an icon or none do

### Bottom hint bar (context-sensitive keybinds)

**Current:** absent / inconsistent across tabs.

**Proposed (mandatory):** every tab renders a 1-line bottom hint bar showing available keybinds for the current view. Examples:
- chat: `Enter send  ↑/↓ scroll  Esc clear input  Ctrl+L clear chat  / commands`
- office: `f focus  m memo  ? help  R reconnect  Esc clear focus`
- pantheon: `n new mission  p pause  c cancel  Enter open  Esc back`
- settings: `↑/↓ navigate  Enter edit  Esc back  ? help`

### Color theme (today, dark)

Current: Dark background, fire-orange accent (matches WebUI launch page + ZeusLabs branding).

Improvements:
- Make the accent color configurable in settings (some terminals render fire-orange poorly with low contrast)
- Add a high-contrast mode for accessibility (defaults to current; togglable)

### Command palette (proposed new feature)

`Ctrl+K` (or `:` for vim-mode users) opens a fuzzy-search command palette. Operations:
- Switch tab by name
- Run a tool with prompt-driven arg input
- Open a settings field directly by name
- Trigger a slash command
- Invoke a skill

This is a launch-friendly feature — single keybind, low surface, dramatically improves discoverability for power users. Could ship pre-launch if desk is clear.

---

## Discovered constraints + regressions (today)

### 1. Office TUI canvas was rendering at stale 80×40 (FIXED today)

`93d88623` fixed by making `office::render` self-heal on every frame against live `area`. Pre-fix, canvas occupied ~30% of terminal regardless of size. ✅ Fixed.

### 2. Office channel-agents fetch broken (in flight — zeus-spark Track B)

TUI polls `/v1/network/agents` (sparse return), should poll `/v1/agents/discover` (full fleet). Same field-name mismatch as WebUI office (just fixed via `8667a72f`). Pending merge.

### 3. Pantheon tab is a 24-line stub

Critical platform feature with no TUI surface. Post-launch sprint.

### 4. Heartbeat noise (`[Plan Resume]`, `[Heartbeat] hourly-N`)

Not strictly TUI-side, but visible in chat history when channel-source messages arrive. Just fixed today via `4cf61332` (plan-resume gating) — agents need redeploy to pick it up.

### 5. CLAUDE.md `TUI Screens` section claims 23 screens

Stale. Actual code has 4 tabs. CLAUDE.md should be updated alongside this PRD's adoption, OR replaced with a TUI architecture doc that stays current.

---

## Design improvement priorities (for this sprint)

| # | Improvement | Tab | Impact | Effort |
|---|-----------|-----|--------|--------|
| 1 | Bottom hint bar (context-sensitive keybinds) | all | High UX consistency | Medium |
| 2 | Settings grouped 2-pane layout | settings | Operator-friendly | Medium |
| 3 | Slash-command overlay in chat | chat | Discoverability | Low |
| 4 | Office agent-focus panel + zone affordances | office | Already-fixed regressions polish | Medium |
| 5 | Color-coded connection state in top bar | global | Quick-glance status | Low |
| 6 | Per-message provider badge in chat | chat | Fallback-chain debug | Low |
| 7 | Pantheon stub → real implementation | pantheon | Critical platform feature surfaced | High (post-launch) |
| 8 | Command palette (`Ctrl+K`) | global | Power-user shortcut | Medium |
| 9 | Tab-bar unread indicators | global | Don't miss channel pings | Low |
| 10 | Re-run onboarding entry in settings | settings | Easier reconfig | Low |

---

## Open questions for merakizzz

1. **Should the Pantheon tab be removed from the TUI for launch** since it's a 24-line stub? Or kept as a "Coming soon" placeholder? Showing a stub may damage credibility on day-1.
2. **Office tab — keep the pixel art Phaser-style aesthetic, or shift to a more abstract presence indicator?** Current art is opinionated; some operators may prefer a tabular agent list.
3. **Settings groups:** my proposed grouping (LLM / Channels / Memory / Security / Tools / Display) — accept or rework?
4. **Command palette:** `Ctrl+K` vs `:` — vim users prefer `:`, other users expect `Ctrl+K` (VS Code, Slack, etc.). Default + alternative?
5. **Slash commands:** which slash commands ship in chat? Today the agent loop accepts `/help`, `/clear`, `/compact`, `/spawn`, `/stop`, `/reset`. The full list isn't curated. Want me to enumerate from `crates/zeus-agent/`?
6. **Color theme switch:** ship a single dark theme for launch, or expose theme picker (dark/high-contrast)?

---

## Status / next steps

- [x] PRD drafted (this doc)
- [ ] merakizzz design pass + open-questions resolution
- [ ] Per-tab implementation dispatch — chat (improvements 3, 6), office (4 polish), settings (2), global (1, 5, 9)
- [ ] Pantheon TUI implementation — separate post-launch sprint, large
- [ ] Command palette — possible pre-launch if desk is clear, else post-launch
- [ ] Update CLAUDE.md TUI section to reflect actual 4-tab structure
