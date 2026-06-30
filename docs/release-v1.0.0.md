# Zeus v1.0.0 — Release Announcement

**Zeus 1.0 is out.**

After six sprints of building, auditing, and hardening, the platform is production-ready. This release ships everything: a 31-crate Rust workspace, six native frontends, a full multi-agent fleet, 212 tools, 9 messaging channels, and a cognitive engine that actually works.

---

## What's in 1.0

### The numbers
- **~342,000 lines of Rust** across 31 crates
- **~17,000 lines of Swift** across macOS, iOS, and visionOS apps
- **6,551 tests**, 0 clippy warnings
- **212 tools** across 22 categories
- **9 channel adapters** (Telegram, Discord, Slack, Matrix, Signal, Email, WhatsApp, MQTT, Mattermost)
- **6 native frontends** (TUI, Web, macOS, iOS, visionOS, Android)

### What changed in S70 (the final sprint before 1.0)

**Config is now consistent.** `config.toml` is the single source of truth. The gateway exports all credentials to env vars at boot — no more silent failures when an env var goes missing after a config edit.

**TUI actually works.** Chat renders in the chat tab (not the log panel). Input is always active — just type and hit Enter. No more `i` to enter insert mode.

**12,780 lines of dead code deleted.** `zeus-prometheus-old/` was a fully superseded crate still sitting in the repo. Gone. Reference copies from the S68 TUI migration — gone. Zero functional impact.

**Agent identity hardened.** The setup wizard requires an explicit agent name. Fleet agents know their own Discord tags and respond correctly to `@mentions`.

---

## Key capabilities

### Pantheon multi-agent orchestration
Give Zeus a goal in plain English. It decomposes it into tasks, assembles a team from the registered fleet, delegates work, monitors progress via SSE, and replans if something fails. Multi-machine, real agents, real coordination.

### 193 macOS automation tools
Calendar, reminders, notes, contacts, Safari, Mail, Music, iMessage, Finder, system control, UI automation, PDF, Bluetooth, network, Homebrew — all scriptable from the agent loop. Plus 11 Chrome DevTools Protocol tools for browser automation.

### Memory that persists
SQLite FTS5 + vector hybrid search (BM25 + cosine). Cross-encoder reranking. Embedding provider fallback chain. Auto-compaction with fact-checking. Sessions indexed for retrieval. Works offline with local Ollama embeddings.

### Six frontends, one gateway
TUI for the terminal. Leptos WASM web app. SwiftUI on macOS, iOS, and visionOS. Jetpack Compose on Android. All connect to the same gateway API. The TUI is the default — `zeus` to launch.

### Security
macOS Seatbelt sandboxing, credential vault (OS keychain + config fallback), SSRF protection, secret redaction, audit logging. All P0 security issues from the S69 audit are closed.

---

## Install

```bash
# Homebrew
brew install zeuslabai/zeus/zeus

# From source
git clone https://github.com/zeuslabai/Zeus.git
cd zeus && cargo build --release

# First run
zeus onboard
```

---

## What's next

- TUI onboarding rebuild (18-step wizard with categorized skills picker)
- WebUI full implementation
- Voice Wake always-on mode
- Canvas / A2UI agent workspace
- Linux desktop app (GTK4/Tauri)
- Federated memory sync

---

[GitHub](https://github.com/zeuslabai/Zeus) · [CHANGELOG](../CHANGELOG.md) · [Docs](https://docs.zeuslab.ai)
