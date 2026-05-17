# Personalities

Character-first AI personalities for Zeus agents. Each personality defines *who the agent is* — voice, values, worldview, and default behavior.

## What This Is

A personality is a persona definition applied at the agent level to shape tone, communication style, and cognitive approach. Unlike task-specific personas (see `workspace/personas/`), personalities are durable identities — they persist across sessions and inform how an agent engages with everything it does.

## Structure

```
personalities/
├── README.md              ← you are here
├── <category>/
│   └── the-<name>.md      ← categorized personalities (canonical format)
├── analyst.md             ← legacy loose files (pending reconciliation)
├── collaborator.md
├── executor.md
└── explorer.md
```

### Categories

| Directory     | Focus                                      | Personalities                                      |
|---------------|--------------------------------------------|-----------------------------------------------------|
| `creative/`   | Expression, storytelling, ideation         | the-herald, the-spark                               |
| `data/`       | Analysis, pattern recognition              | the-oracle                                          |
| `design/`     | Visual thinking, UX, aesthetics            | the-visionary                                       |
| `devops/`     | Infrastructure, reliability, automation    | the-plumber                                         |
| `engineering/`| Systems design, code architecture         | the-architect, the-builder, the-executor, the-operator |
| `fullstack/`  | Cross-layer generalism                     | the-polyglot                                        |
| `general/`    | Minimal footprint, versatility             | the-minimalist                                      |
| `marketing/`  | Messaging, growth, audience thinking       | the-amplifier                                       |
| `mobile/`     | Native platforms, device-first thinking    | the-crafter                                         |
| `product/`    | Strategy, user focus, prioritization       | the-analyst, the-partner                            |
| `research/`   | Deep inquiry, synthesis, citation rigor    | the-scholar                                         |
| `security/`   | Threat modeling, defense, risk             | the-sentinel                                        |
| `trading/`    | Markets, signals, execution                | the-market-analyst, the-trader                      |

## File Schema

Canonical personality files use this frontmatter:

```yaml
---
name: The <Name>
tagline: <short description>
category: <Category>
default_skills: [skill1, skill2, ...]
---
```

Followed by free-form prose defining voice and behavior.

## Naming Convention

- Categorized files: `the-<name>.md` inside a category directory
- Names use the definite article ("The Architect") to signal archetype, not job title

## Loose Files (Legacy)

All legacy root-level files have been reconciled (T10):

| File             | Disposition                                         |
|------------------|-----------------------------------------------------|
| `analyst.md`     | Deleted — superseded by `product/the-analyst.md`    |
| `collaborator.md`| Deleted — placeholder content, no real value        |
| `executor.md`    | Migrated to `engineering/the-executor.md`           |
| `explorer.md`    | Deleted — placeholder content, no real value        |

## Related

- `workspace/personas/` — task-scoped agent configs (different purpose, different format)
