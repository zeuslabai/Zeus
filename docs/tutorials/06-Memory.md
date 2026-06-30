# Memory System

Zeus has a layered memory system: workspace files for structured context, Mnemosyne for searchable long-term storage, and daily notes for journaling.

## Workspace Files

The workspace at `~/.zeus/workspace/` is read on every interaction:

| File | Purpose | What to Put Here |
|------|---------|-----------------|
| `AGENTS.md` | System prompt | Define Zeus's role, capabilities, personality |
| `SOUL.md` | Style guide | Communication style, tone preferences |
| `USER.md` | User context | Your name, projects, tech stack, preferences |
| `HEARTBEAT.md` | Proactive tasks | Tasks for the heartbeat loop to process |
| `memory/MEMORY.md` | Long-term facts | Accumulated knowledge across sessions |

### Editing Workspace Files

You can edit these directly or via CLI:

```bash
# View current workspace
zeus memory show

# Add a long-term fact
zeus memory remember "I use a MacBook Pro M4 with 64GB RAM"

# Add a daily note
zeus memory note "Deployed v2.0 to production today"
```

Or just ask Zeus in chat:

```
"Remember that my project uses PostgreSQL 16 and runs on port 5432"
```

Zeus will store this in `MEMORY.md` and recall it in future sessions.

## Mnemosyne (Advanced Memory)

Mnemosyne provides SQLite-backed full-text search and vector embeddings for semantic recall.

### Enable It

```toml
# ~/.zeus/config.toml
[mnemosyne]
db_path = "~/.zeus/memory.db"
enable_fts = true
```

### How It Works

1. **FTS5 Full-Text Search** — Zeus indexes conversations and facts using SQLite FTS5. Fast keyword-based retrieval.
2. **Vector Embeddings** — Optional. Uses Ollama (`nomic-embed-text`) or OpenAI embeddings for semantic similarity search.
3. **Hybrid Search** — Combines BM25 keyword scoring with cosine vector similarity for best results.

### Searching Memory

Via API:

```bash
curl -X POST http://localhost:3001/v1/memory/search \
  -H "Content-Type: application/json" \
  -d '{"query":"database configuration","mode":"hybrid"}'
```

Search modes: `"fts"` (keyword only), `"vector"` (semantic only), `"hybrid"` (both).

### Embedding Host Pinning

If you have a GPU server, pin embeddings there:

```toml
[mnemosyne]
embedding_host = "http://gpu-server:11434"
```

This lets your local Zeus use a remote Ollama instance specifically for embedding generation.

## Daily Notes

Zeus automatically creates daily notes at `~/.zeus/workspace/daily/YYYY-MM-DD.md`:

```bash
# Add a note for today
zeus memory note "Fixed the authentication bug in the user API"
```

Notes accumulate throughout the day, creating a journal of your work.

## Memory in Practice

### Example: Project Context

Edit `~/.zeus/workspace/USER.md`:

```markdown
## Projects

### MyApp
- Stack: Rust + Axum + PostgreSQL
- Repo: ~/projects/myapp
- Port: 3000 (dev), 8080 (prod)
- Database: PostgreSQL 16 on localhost:5432

### Preferences
- Editor: Neovim
- Shell: zsh
- Style: Prefer concise, well-commented code
```

Now every Zeus interaction will have this context. Ask "What port does MyApp use?" and Zeus knows.

### Example: Accumulated Facts

After several sessions of `zeus memory remember`:

```markdown
# ~/.zeus/workspace/memory/MEMORY.md

## Facts
- User prefers Python for scripting, Rust for systems
- Production server is at 192.168.1.100
- SSH key is at ~/.ssh/id_ed25519
- API rate limit is 1000 req/min
- Database backup runs at 3am UTC
```

### Auto-Compaction

Mnemosyne automatically compacts memory with fact-checking:
- Detects duplicate or contradictory facts
- Merges related information
- Removes outdated entries (with confirmation)

## What's Next

→ [[07-TUI]] — Interactive terminal interface
→ [[18-Cognitive-Engine]] — How Zeus learns from interactions
