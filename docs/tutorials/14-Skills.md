# Skills

Zeus has a skills ecosystem compatible with the OpenClaw SKILL.md format. Skills are reusable capability descriptions that Zeus can auto-activate based on context.

## What Are Skills?

A skill is a Markdown file (`SKILL.md`) that describes a capability — how to use a tool, API, or workflow. Zeus reads skills and injects them into the LLM context when relevant.

Zeus ships with **52 built-in skills** covering git, Docker, databases, coding practices, and more.

## Browsing Skills

### CLI

```bash
zeus chat "What skills do you have?"
```

### API

```bash
# List all skills
curl http://localhost:3001/v1/skills | jq

# Search skills
curl "http://localhost:3001/v1/skills/search?q=docker" | jq

# Get skill details
curl http://localhost:3001/v1/skills/<id> | jq

# List categories
curl http://localhost:3001/v1/skills/categories | jq
```

### TUI

Press `9` (Extensions screen) to browse installed skills.

## Built-in Skills

Zeus includes skills in the `skills/` directory:

| Skill | Description |
|-------|-------------|
| `git` | Git operations, branching, rebasing |
| `docker` | Container management, Compose |
| `kubectl` | Kubernetes cluster management |
| `postgres` | PostgreSQL administration |
| `redis` | Redis cache operations |
| `ssh` | SSH connections and tunneling |
| `sqlite` | SQLite database management |
| `homebrew` | macOS package management |
| `email-client` | Email composition and management |
| `discord-cli` | Discord bot and API usage |
| `slack-cli` | Slack app and API usage |
| `obsidian` | Obsidian vault management |
| `notion` | Notion API integration |
| `jira` | Jira issue tracking |
| `linear` | Linear project management |
| `code-review` | Code review best practices |
| `security-review` | Security audit checklist |
| `tdd` | Test-driven development |
| `technical-docs` | Technical documentation |
| `markdown` | Markdown formatting |
| `plan` | Project planning |
| `orchestrate` | Multi-agent orchestration |
| `evolve` | Self-improvement |
| `learn` | Learning from interactions |
| `verify` | Output verification |
| `checkpoint` | Progress checkpointing |
| `build-fix` | Build error resolution |
| `rsync` | File synchronization |
| `bun` | Bun JavaScript runtime |

## Skill Format (SKILL.md)

Skills follow the OpenClaw SKILL.md format:

```markdown
---
name: my-skill
version: 1.0.0
description: What this skill does
category: development
tags: [rust, coding]
emoji: 🦀
read_when:
  - user asks about Rust programming
  - files with .rs extension are involved
requirements:
  - name: rustc
    check: rustc --version
install:
  - brew install rust
---

# My Skill

## Instructions

When the user asks about Rust programming:

1. Prefer idiomatic Rust patterns
2. Use `Result<T, E>` for error handling
3. Prefer `&str` over `String` for function parameters
4. Add `#[derive(Debug)]` to custom types

## Examples

### Creating a new project
```bash
cargo new my-project
cd my-project
cargo run
```
```

## Key Fields

| Field | Description |
|-------|-------------|
| `name` | Unique skill identifier |
| `version` | Semantic version |
| `description` | Short description |
| `category` | Grouping (development, ops, etc.) |
| `tags` | Search tags |
| `emoji` | Display emoji |
| `read_when` | Auto-activation triggers — when these conditions match, the skill is injected into context |
| `requirements` | System dependencies to check |
| `install` | Commands to install dependencies |

## Auto-Activation (read_when)

The `read_when` field is powerful — it tells Zeus when to automatically include this skill in the LLM context:

```yaml
read_when:
  - user asks about Docker containers
  - Dockerfile is present in the project
  - docker-compose.yml is referenced
```

When any trigger matches, Zeus reads the skill and includes its instructions in the system prompt.

## Creating Your Own Skills

1. Create a directory in `~/.zeus/skills/` (or configure a custom path)
2. Add a `SKILL.md` file following the format above
3. Zeus discovers it automatically on next interaction

### Example: Custom Team Skill

```markdown
---
name: team-conventions
version: 1.0.0
description: Our team's coding conventions
category: development
read_when:
  - user asks about coding style
  - pull request review
---

# Team Conventions

## Code Style
- 4-space indentation
- Max line length: 100 chars
- All public functions must have doc comments
- Tests required for all new features
```

## Agora Marketplace

The Agora marketplace allows agents to trade skills:

```bash
# Browse marketplace
curl http://localhost:3001/v1/pantheon/economy | jq

# Slash commands in War Room
/skills            # List available skills
/search <query>    # Search skills
/publish <name>    # Publish a skill
/buy <name>        # Purchase a skill
```

## What's Next

→ [[15-Security]] — Security sandbox and permissions
→ [[13-Pantheon]] — Multi-agent missions
