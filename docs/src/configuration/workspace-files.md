# Workspace Files

The Zeus workspace is a directory of markdown files that provide persistent context to the agent. By default, the workspace is located at `~/.zeus/workspace/`.

## Directory Layout

```
~/.zeus/workspace/
├── AGENTS.md          # System prompt
├── SOUL.md            # Personality definition
├── USER.md            # User context and preferences
├── HEARTBEAT.md       # Proactive task definitions
├── memory/
│   └── MEMORY.md      # Long-term facts and knowledge
└── daily/
    └── YYYY-MM-DD.md  # Daily notes (one file per day)
```

## File Descriptions

### AGENTS.md

The system prompt that defines Zeus's behavior and capabilities. This file is loaded at the start of every agent interaction and sets the foundational instructions for the LLM. Edit this file to customize how Zeus approaches tasks, what it prioritizes, and what constraints it follows.

### SOUL.md

The personality definition for the agent. This file shapes the tone, style, and character of Zeus's responses. It is injected into the system prompt alongside `AGENTS.md` to give the agent a consistent voice.

### USER.md

User-specific context and preferences. This file contains information about you -- your name, role, common projects, preferred workflows, and any other context that helps Zeus provide relevant responses. Zeus may update this file as it learns about you.

### HEARTBEAT.md

Proactive task definitions for the Prometheus heartbeat system. When the gateway daemon is running, Prometheus periodically reads this file to identify tasks that should be executed without user prompting. Use this for recurring checks, monitoring, or background work.

### memory/MEMORY.md

Long-term memory storage. Facts and knowledge are appended here via the `zeus memory remember "fact"` command or when the agent uses the `memory_remember` capability during conversation. Each entry is timestamped.

Example content:

```markdown
- The project uses PostgreSQL 15 on production
- Deployment happens via GitHub Actions to AWS ECS
- The main API is at api.example.com
```

### daily/YYYY-MM-DD.md

Daily note files, one per day, created automatically when you use `zeus memory note "content"` or when the agent logs daily activity. These provide a chronological record of interactions and notes.

Example filename: `daily/2026-02-11.md`

## Customizing the Workspace Path

The workspace location can be changed in `config.toml`:

```toml
workspace = "~/my-custom-workspace"
```

Zeus creates the directory structure automatically on first use if it does not exist.

## How Workspace Files Are Used

1. On each agent interaction, `AGENTS.md`, `SOUL.md`, and `USER.md` are loaded into the system prompt.
2. The Nous cognitive engine adds cognitive context (intent, reasoning) to the prompt.
3. Mnemosyne searches `MEMORY.md` and past conversations for relevant context.
4. The combined context is sent to the LLM along with the user's message.
5. After the interaction, new facts and notes are persisted back to the workspace.
