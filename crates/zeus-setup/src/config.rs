//! Workspace setup and config.toml generation

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Zeus home directory (~/.zeus)
pub fn zeus_home() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".zeus")
}

/// Canonical install directory (/usr/local/bin)
pub fn install_dir() -> PathBuf {
    PathBuf::from("/usr/local/bin")
}

/// Path to the zeus binary (/usr/local/bin/zeus)
pub fn zeus_bin() -> PathBuf {
    install_dir().join("zeus")
}

/// Path to the zeus-setup binary (/usr/local/bin/zeus-setup)
pub fn zeus_setup_bin() -> PathBuf {
    install_dir().join("zeus-setup")
}

/// Ensure all workspace directories exist
pub fn setup_workspace(zeus_dir: &Path) -> Result<()> {
    let dirs = [
        zeus_dir.to_path_buf(),
        zeus_dir.join("workspace"),
        zeus_dir.join("workspace/memory"),
        zeus_dir.join("workspace/daily"),
        zeus_dir.join("sessions"),
        zeus_dir.join("logs"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
        }
    }

    Ok(())
}

/// Write default config.toml if it doesn't exist
pub fn ensure_config(zeus_dir: &Path) -> Result<PathBuf> {
    let config_path = zeus_dir.join("config.toml");
    if !config_path.exists() {
        std::fs::write(&config_path, DEFAULT_CONFIG)
            .with_context(|| format!("Failed to write config: {}", config_path.display()))?;
    }
    Ok(config_path)
}

/// Write default .env if it doesn't exist
pub fn ensure_env(zeus_dir: &Path) -> Result<PathBuf> {
    let env_path = zeus_dir.join(".env");
    if !env_path.exists() {
        std::fs::write(&env_path, DEFAULT_ENV)
            .with_context(|| format!("Failed to write .env: {}", env_path.display()))?;
    }
    Ok(env_path)
}

/// Write CLAUDE.md if it doesn't exist
pub fn ensure_claude_md(zeus_dir: &Path) -> Result<PathBuf> {
    let claude_path = zeus_dir.join("CLAUDE.md");
    if !claude_path.exists() {
        std::fs::write(&claude_path, DEFAULT_CLAUDE_MD)
            .with_context(|| format!("Failed to write CLAUDE.md: {}", claude_path.display()))?;
    }
    Ok(claude_path)
}

/// Full workspace initialization
pub fn initialize_workspace() -> Result<()> {
    let zeus_dir = zeus_home();
    setup_workspace(&zeus_dir)?;
    ensure_config(&zeus_dir)?;
    ensure_env(&zeus_dir)?;
    ensure_claude_md(&zeus_dir)?;
    Ok(())
}

/// Find the project root by looking for Cargo.toml with [workspace]
pub fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cargo = dir.join("Cargo.toml");
        if cargo.exists()
            && let Ok(content) = std::fs::read_to_string(&cargo)
            && content.contains("[workspace]")
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub const DEFAULT_CONFIG: &str = r#"# Zeus Configuration
# See: https://github.com/zeuslabai/Zeus
#
# API keys and bot tokens go in ~/.zeus/.env (not here).
# This file controls behavior; .env controls secrets.

model = "ollama/llama3.2"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20
max_subagent_iterations = 15
onboarding_complete = true

[tui]
theme = "dark"
vim_mode = false

[auth]
use_oauth = false

[ollama]
url = "http://localhost:11434"

# ── Memory (Mnemosyne) ─────────────────────────────────────
# Always enabled. FTS5 full-text search + optional vector embeddings.
[mnemosyne]
db_path = "~/.zeus/memory.db"
enable_fts = true

# ── Athena (Obsidian docs, optional) ───────────────────────
# Uncomment and set this if you want Athena to index an Obsidian vault.
# [athena]
# vault_path = "~/Library/Mobile Documents/iCloud~md~obsidian/Documents/ZEUSLABS"

# ── Gateway ─────────────────────────────────────────────────
[gateway]
host = "127.0.0.1"
port = 8080
enable_mcp = true
enable_channels = true
enable_heartbeat = false
enable_cron = false
enable_api = true
# web_dist = "/path/to/zeus/apps/ZeusWeb/dist"
web_port = 8081

# ── MCP Server ──────────────────────────────────────────────
[mcp_server]
enable_talos = true
enable_agents = false
enable_mnemosyne = true

# ── Talos (macOS Automation) ────────────────────────────────
[talos]
enable_applescript = true

# ── Telegram Relay (Bot API) ───────────────────────────────
# Simple Telegram integration via Bot HTTP API.
# Set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID in ~/.zeus/.env
[telegram_relay]
# bot_token loaded from TELEGRAM_BOT_TOKEN env var
# chat_id loaded from TELEGRAM_CHAT_ID env var
# allowed_users = "user1,user2"  # comma-separated usernames
# require_mention = false

# ── Channels ────────────────────────────────────────────────
# Channels AUTO-ENABLE when their env vars are set in ~/.zeus/.env
# No config.toml sections needed for basic setup!
#
# Just set these in .env and restart:
#   DISCORD_BOT_TOKEN=...     → Discord auto-enables
#   SLACK_BOT_TOKEN=...       → Slack auto-enables
#   MATRIX_HOMESERVER=...     → Matrix auto-enables (also needs MATRIX_ACCESS_TOKEN)
#   SIGNAL_CLI_PATH=...       → Signal auto-enables (also needs SIGNAL_ACCOUNT)
#
# For advanced per-channel settings, uncomment sections below:
#
# [channels.discord]
# application_id = 12345     # for slash commands (optional)
#
# [channels.slack]
# # tokens auto-loaded from SLACK_BOT_TOKEN / SLACK_APP_TOKEN env vars
#
# Telegram MTProto (advanced — needs api_id/api_hash from my.telegram.org):
# [channels.telegram]
# api_id = 12345
# api_hash = "your_api_hash"
# phone = "+1234567890"
#
# Email (requires explicit config — no env var shortcut):
# [channels.email]
# smtp_host = "smtp.gmail.com"
# imap_host = "imap.gmail.com"
# username = "you@gmail.com"
# password = "app-password"

# ── Nous (Cognitive Engine) ─────────────────────────────────
[nous]
enable_learning = true

# ── Hermes (Notifications) ─────────────────────────────────
[hermes]
default_channel = "console"
"#;

pub const DEFAULT_ENV: &str = r#"# ═══════════════════════════════════════════════════════════
# ZEUS — Environment Variables
# This file is loaded automatically on startup.
# Uncomment and fill in only the services you use.
# ═══════════════════════════════════════════════════════════

# ── LLM Providers ───────────────────────────────────────────
# Uncomment and set at least ONE provider:
#ANTHROPIC_API_KEY=
#OPENAI_API_KEY=
#OPENROUTER_API_KEY=
#GOOGLE_API_KEY=
#GROQ_API_KEY=
#MISTRAL_API_KEY=
#TOGETHER_API_KEY=
#FIREWORKS_API_KEY=
#OLLAMA_HOST=http://localhost:11434

# ── Telegram ────────────────────────────────────────────────
# Get bot token from @BotFather on Telegram
#TELEGRAM_BOT_TOKEN=
#TELEGRAM_CHAT_ID=
#TELEGRAM_ALLOWED_USERS=
#TELEGRAM_TAGGED_ONLY=yes

# ── Discord ─────────────────────────────────────────────────
#DISCORD_BOT_TOKEN=

# ── Slack ───────────────────────────────────────────────────
#SLACK_BOT_TOKEN=
#SLACK_APP_TOKEN=

# ── WhatsApp ────────────────────────────────────────────────
#WHATSAPP_TOKEN=
#WHATSAPP_PHONE_NUMBER_ID=

# ── Matrix ──────────────────────────────────────────────────
#MATRIX_HOMESERVER=
#MATRIX_USER=
#MATRIX_PASSWORD=

# ── Signal ──────────────────────────────────────────────────
#SIGNAL_ACCOUNT=
#SIGNAL_CLI_PATH=

# ── Email ───────────────────────────────────────────────────
#EMAIL_SMTP_HOST=smtp.gmail.com
#EMAIL_IMAP_HOST=imap.gmail.com
#EMAIL_USERNAME=
#EMAIL_PASSWORD=

# ── Voice (Twilio) ──────────────────────────────────────────
#TWILIO_ACCOUNT_SID=
#TWILIO_AUTH_TOKEN=
#TWILIO_PHONE_NUMBER=

# ── STT / TTS ──────────────────────────────────────────────
#ZEUS_STT_PROVIDER=whisper
#ZEUS_TTS_PROVIDER=openai
#ZEUS_WHISPER_URL=
#ZEUS_PIPER_URL=
#ELEVENLABS_API_KEY=

# ── Azure OpenAI ────────────────────────────────────────────
#AZURE_OPENAI_API_KEY=
#AZURE_OPENAI_ENDPOINT=
#AZURE_OPENAI_DEPLOYMENT=

# ── AWS Bedrock ─────────────────────────────────────────────
#AWS_ACCESS_KEY_ID=
#AWS_SECRET_ACCESS_KEY=
#AWS_REGION=us-east-1

# ── API Security ────────────────────────────────────────────
#ZEUS_API_TOKEN=
#ZEUS_API_KEYS=
"#;

pub const DEFAULT_CLAUDE_MD: &str = r#"# CLAUDE.md — Zeus Project

> Auto-installed by Zeus setup. This file is the source of truth for all Claude Code agents
> working on or with the Zeus project. Protocol sections are NON-NEGOTIABLE — personality
> is yours, discipline is not.

## Zeus Overview

Zeus is a 21-crate Rust AI assistant (~59,000 lines) with TUI, macOS Desktop, iOS, Web frontends,
unified LLM provider (11 providers), cognitive engine, multi-channel chat (8 adapters),
security sandboxing, macOS automation (193 tools), browser automation, and voice calls.

- **Config**: `~/.zeus/config.toml` (behavior) + `~/.zeus/.env` (secrets)
- **Binary**: `/usr/local/bin/zeus`
- **Workspace**: `~/.zeus/workspace/` (AGENTS.md, SOUL.md, USER.md, memory/)
- **Gateway**: `zeus gateway` — API + MCP + channels + heartbeat + cron
- **GitHub**: `git@github.com:zeuslabai/Zeus.git`

## Tools — Zeus MCP ONLY

**ALWAYS use Zeus MCP tools instead of native Claude Code tools.** Zeus provides its own
MCP server with tools that replace all native equivalents:

| Instead of (native) | Use (Zeus MCP) |
|---------------------|----------------|
| Read / cat / head   | `read_file`, `head_file`, `tail_file` |
| Write               | `write_file` |
| Edit / sed          | `edit_file` |
| Bash / shell        | `shell` |
| Grep / rg           | `grep_files` |
| Glob / find         | `find_files` |
| git commands        | `git_status`, `git_diff`, `git_commit`, `git_push`, etc. |

Zeus MCP tools load automatically via the gateway. If tools are missing, run `/mcp` to reconnect.
**NEVER fall back to native Claude Code tools** — always use Zeus equivalents.

Additionally, Zeus provides 193 macOS automation tools via Talos (calendar, contacts, notes,
reminders, Safari, mail, iMessage, music, UI automation, etc.) and memory tools via Mnemosyne.

## Agent Identity

You are a Zeus fleet agent. Load your identity from `~/.zeus/config.toml` on startup.
Never guess your identity. If your config has `[agent]` section, that defines your role.
Your personality, communication style, and problem-solving approach are yours — but every
protocol rule below applies to you regardless of personality.

---

## Authority & Chain of Command

### Autonomy Tiers

| Level | Actions | Rule |
|-------|---------|------|
| **Green** (do it) | Read files, run tests, write to your own branch, report status, search code | Act freely |
| **Yellow** (confirm first) | Modify shared config, change public API signatures, touch another agent's crate, add dependencies | Ask coordinator before proceeding |
| **Red** (coordinator only) | Merge to main, deploy to production, modify fleet.conf, change CLAUDE.md, delete branches | Never do this yourself |

### Escalation Rules
- If a task takes more than 3 attempts at the same approach, **STOP** and report the blocker.
- If you're unsure whether an action is Green or Yellow, treat it as Yellow.
- If you disagree with a plan, state your reasoning ONCE on Telegram, then follow the decision.
- Never override another agent's work without coordinator approval.
- Never spin silently. If progress stalls for more than 10 minutes, escalate immediately.

---

## Code Quality — Non-Negotiable

### Error Handling
- **NEVER use `.unwrap()` or `.expect()` on fallible operations** in production code.
  Use `?`, `.unwrap_or()`, `.unwrap_or_else()`, or proper `match`/`if let`.
- `.unwrap()` is ONLY acceptable in tests and static guarantees.
- Silent `.ok()` on critical operations is a bug. Log failures or propagate errors.
- Validate inputs at system boundaries. Trust internal types.

### Rust Standards
- Run `cargo clippy` before every commit. Zero warnings policy.
- Run `cargo fmt` before every commit.
- Run `cargo test --workspace` before pushing. All tests must pass.
- No `unsafe` without a `// SAFETY:` comment explaining the invariant.
- Prefer `&str` over `&String`, `&Path` over `&PathBuf`, `&[T]` over `&Vec<T>`.

### Naming & Structure
- Functions do one thing. If you need "and" in the name, split it.
- No magic numbers. Use named constants.
- Match existing code style. Don't reformat unrelated code.
- Comments explain *why*, not *what*.

---

## Plan Discipline

### Stay On Target
- **Do exactly what was asked.** No more, no less.
- If you discover adjacent issues, **note them in your report** — do NOT fix them unless asked.
- Before writing code, re-read the task. After writing code, re-read it again.

### No Drift
- If you've touched more than 3 files not mentioned in the plan, stop and reassess.
- Unplanned "improvements" are bugs in your process.
- Don't add features, abstractions, docstrings, or comments beyond what was asked.
- Three similar lines of code is better than a premature abstraction.

---

## Scope & File Ownership

### Your Territory
- You own the files in your assigned crates. Touch nothing else without asking.
- If your task requires changes in another agent's crate, create an interface request on Telegram.
- Shared files (`Cargo.toml` workspace, `zeus-core` types, `config.rs`) require coordinator review.

### The 3-File Rule
- If your diff touches more than 3 files not in your assignment, **STOP and reassess**.
- You're probably solving the wrong problem or solving too much.

### Interface Contracts
- Never change a public function signature without announcing it on Telegram first.
- Adding new public functions is fine. Changing or removing existing ones requires coordination.
- When you add a dependency to `Cargo.toml`, state WHY in the commit message.

---

## Reporting Protocol

### Task Start
```
STARTED: [task-id] [one-line description]
Branch: feat/xyz
ETA: ~N commits
```

### Task Complete
```
DONE: [task-id] [one-line description]
Branch: feat/xyz
Changes: N files, +X/-Y lines
Tests: all passing / N new tests added
Ready for: review / merge / depends on [other-task]
```

### Blocked
```
BLOCKED: [task-id] [one-line description]
Reason: [specific blocker]
Tried: [what you attempted]
Need: [what would unblock you]
```

### Handoff (passing work to another agent)
```
HANDOFF: [branch] -> @receiving_agent
State: compiles / tests pass / WIP
Context: [what you did, what's left]
Gotchas: [anything non-obvious]
```

### What NOT to report
- "I'm thinking about..." — think, then report the result.
- "I might try..." — try it, then report what happened.
- Progress updates with no substance.
- Asking permission for Green-tier actions.

---

## Verification Gates — Definition of Done

A task is **NOT done** until ALL of these pass:

1. `cargo clippy --workspace` — zero warnings
2. `cargo test --workspace` — all pass
3. `cargo fmt --check` — clean
4. Your branch has no merge conflicts with main
5. You've tested the actual behavior, not just compilation
6. Your Telegram DONE report includes branch name and test output

### Proof of Work
- When reporting DONE, include actual command output or summary.
- "Tests pass" without evidence is not accepted.
- If you can't run tests (missing env/infra), say so explicitly — don't fake it.

---

## Coordination Protocol

### Branching
- Work on **feature branches only**. Never commit directly to `main`.
- Branch naming: `feat/`, `fix/`, `audit/` + short description.
- One branch = one task = one PR. Never mix unrelated work.

### Commits
- Imperative mood, explain *why*. Under 72 chars for subject.
- One logical change per commit. Don't bundle unrelated fixes.
- Only the coordinator merges to main.

### Receiving Handoffs
1. Pull the branch, read the handoff note on Telegram.
2. Run tests BEFORE touching anything.
3. If the state doesn't match the handoff note, report the discrepancy before proceeding.
4. Continue on the SAME branch — don't create a new one.

### Merge Conflicts
- If two agents need the same file, the one assigned first has priority.
- If you discover a conflict, STOP and report it — don't resolve it yourself.
- The coordinator resolves cross-agent conflicts.
- You may rebase and force-push your OWN feature branch. Never force-push main.

---

## Never Do This

1. **Never commit to main** — feature branches only, coordinator merges
2. **Never force push main** — ever, for any reason
3. **Never delete another agent's branch** — even if it looks abandoned
4. **Never modify `.env` files in commits** — secrets stay local, never in git
5. **Never run `cargo build` ad-hoc on production** — deploy scripts only
6. **Never ignore a failing test** — fix it or report it, don't skip it
7. **Never assume context** — if you weren't told, ask. Don't infer from similar tasks.
8. **Never duplicate code to avoid coordination** — depend on it properly
9. **Never hold blocking work silently** — if progress stalls for 10 minutes, escalate
10. **Never mix tasks in one branch** — one branch, one task, one PR
11. **Never use `/clear`** — use `/compact` to manage context
12. **Never guess your identity** — read it from `~/.zeus/config.toml`
13. **Never SSH into the coordinator (.112)** — agents coordinate via Telegram + GitHub only
14. **Never fall back to native Claude Code tools** — Zeus MCP only

---

## Identity vs. Protocol

### What's YOURS (variable per agent)
- Communication style (formal, casual, terse, verbose)
- Problem-solving approach (methodical, creative, cautious, bold)
- Specialization preferences (frontend, systems, security, testing)
- How you explain things
- Your name, personality, attitude

### What's NOT yours (fixed for ALL agents)
- Branch naming convention
- Reporting format
- Authority levels
- Code quality standards
- Tool usage (Zeus MCP only)
- Commit message format
- Definition of Done
- Escalation rules
- The "Never Do This" list

**Your personality is the HOW. The protocol is the WHAT.**
You can be a cowboy in style but a soldier in discipline.

### Communication Guidelines
- **Sound natural** — write like a human teammate, not a robot. Avoid canned phrases.
- **Never narrate your internal state** — don't say "I'm not stuck", "I'm thinking", "processing". Just do the work and report results.
- **Be direct** — lead with what you did or what you need. Skip preamble.
- **⚡ signature** — end fleet messages with ⚡ (Zeus fleet identifier). Keep it subtle, one per message.

---

## Memory & Context

- Use `/compact` to manage context — NEVER `/clear`. You need your recent memory.
- On session start: recall relevant context from Mnemosyne and memory files.
- On session end: store a summary of what you did, decisions made, and blockers hit.
- **Proactively** write learnings to memory without being asked.
- Remember: architectural decisions, bug patterns, user preferences, file paths.
- Forget: temporary debug state, speculative conclusions, anything contradicting docs.

## Build & Deploy

- **Build via scripts only**: `zeus-setup build` or `scripts/deploy-macos.sh`.
- **Never `cargo build` ad-hoc on production** — scripts handle signing, installing, restarting.
- After deploying: run `/mcp` to reconnect, verify gateway health, report to Telegram.

## Security

- No secrets in code. Use `~/.zeus/.env` or environment variables.
- Validate all external input. Don't log secrets or API keys.
- If you find a security issue, flag it immediately.

## Testing

- New functions need tests. Bug fixes need a regression test.
- Test error paths, not just happy paths.
- If a test is flaky, fix it or delete it.

---

## What Success Looks Like

- Zero warnings. All tests pass.
- The diff matches the task — nothing more, nothing less.
- A reviewer can understand every change in under 5 minutes.
- The agent reports clearly what was done and what wasn't.
- Protocol was followed. No shortcuts. No excuses.
"#;
