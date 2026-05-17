# OpenClaw Compatibility

Zeus supports the OpenClaw skill format through the `zeus-skills` crate. Skills defined in SKILL.md files can be loaded, executed, and managed alongside Zeus's built-in tools.

## What is OpenClaw?

OpenClaw is a standard for packaging AI agent capabilities as portable skills. Each skill is defined in a single `SKILL.md` file that contains the skill's name, description, system prompt, tool definitions, and permission requirements. Skills can be shared through ClawHub, a registry for discovering and distributing skills.

## SKILL.md Format

A SKILL.md file follows this structure:

```markdown
# My Skill Name

A brief description of what this skill does.

## Version: 1.0.0

## Author: Your Name

## System Prompt

Instructions for the agent when this skill is active.
These can span multiple lines and define the skill's
behavior, personality, and constraints.

## Tools

- tool_name: Description of what this tool does
- another_tool: Description of this tool

## Permissions

- network: Needs internet access
- filesystem: Needs to read project files
```

### Sections

| Section | Required | Description |
|---------|----------|-------------|
| `# Name` | Yes | The H1 heading is the skill name |
| Description | No | First non-heading paragraph becomes the description |
| `## Version` | No | Semantic version (defaults to `0.1.0`) |
| `## Author` | No | Skill author |
| `## System Prompt` | No | Agent instructions when the skill is active |
| `## Tools` | No | Tool definitions as bullet points (`- name: description`) |
| `## Permissions` | No | Required permissions as bullet points |

## Skill Directory Structure

Skills are stored under `~/.zeus/skills/` (or `~/.zeus/workspace/skills/`), with each skill in its own directory:

```
~/.zeus/skills/
├── code-review/
│   ├── SKILL.md
│   └── review.sh
├── deploy-helper/
│   ├── SKILL.md
│   └── deploy.py
└── data-analysis/
    └── SKILL.md
```

The `SKILL.md` file is required. Additional files in the skill directory (scripts, data, etc.) are available for the skill's tools to reference.

## Tool Implementations

Tools defined in a SKILL.md can be implemented in three ways:

| Type | Description |
|------|-------------|
| **Shell** | A shell command with placeholder substitution. Arguments are sanitized with single-quote escaping before insertion. |
| **Script** | A script run with a specified interpreter (e.g., `python3`, `node`). |
| **Native** | A Rust-native implementation (planned, not yet supported). |

Shell commands support placeholder substitution:

```bash
# If the tool is invoked with {"path": "/tmp/file.txt"}
# The command "cat {path}" becomes "cat '/tmp/file.txt'"
```

Arguments are sanitized to prevent shell injection: single quotes in values are escaped with `'\''`.

## ClawHub Integration

The `ClawHubClient` connects to ClawHub (`https://clawhub.io/api/v1`) for skill discovery and installation:

```rust
let client = ClawHubClient::new();
let results = client.search("code review").await?;
```

Installing a skill from ClawHub downloads the SKILL.md file and any associated assets to the local skills directory:

```rust
skill_manager.install("code-review").await?;
```

## Skill Manager

The `SkillManager` handles loading, listing, executing, and uninstalling skills:

```rust
let mut manager = SkillManager::default();

// Load all skills from ~/.zeus/skills/
let count = manager.load_all().await?;

// List loaded skills
for skill in manager.list() {
    println!("{}: {}", skill.name, skill.description);
}

// Execute a skill's tool
let result = manager.execute("code-review", "analyze", args).await?;

// Get a compact summary for the system prompt
let summary = manager.get_summary();

// Uninstall a skill
manager.uninstall("code-review")?;
```

The `get_summary()` method returns a compact list of available skills suitable for injection into the system prompt, so the agent knows what skills are available without loading their full content.
