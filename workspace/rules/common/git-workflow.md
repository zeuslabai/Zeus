# Git Workflow (Zeus Fleet)

## Branch Naming

```
feat/<sprint>-<description>     # new feature
fix/<description>               # bug fix
housekeeping/<description>      # cleanup, refactor, docs
docs/<description>              # docs only
```

## Commit Message Format

```
<type>: <description>

<optional body>

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`

## Gate Protocol

- **Features**: 4/4 non-author LGTMs before merge to main
- **Housekeeping/fix/docs**: 2/2 non-author LGTMs before merge
- Self-gates do not count
- Branch author cannot gate their own PR
- Use 3-dot diff for gate review: `git diff origin/main...origin/<branch>`

## Pull Request Workflow

1. Analyze full commit history for the branch (not just latest commit)
2. Use `git diff origin/main...origin/<branch>` to see all changes
3. Verify: `cargo test --workspace` + `cargo clippy` + `cargo fmt --check` pass
4. Draft PR summary with: what changed, why, test coverage
5. Post gate request to Discord #private fleet channel
6. Merge only after gate protocol satisfied

## Merge Rules

- Fast-forward preferred for clean history
- No force-push to main
- No `--no-verify` bypass
- Never amend published commits — create a new commit instead
