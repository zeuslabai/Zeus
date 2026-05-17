# Code Review Context

Use this context when reviewing code — PRs, gate reviews, or post-implementation review.

## Active Mode: Code Review

You are in review mode. Be thorough. Flag issues clearly by severity.

## Review Checklist

### Correctness
- [ ] Logic is correct for all inputs including edge cases
- [ ] Error handling is explicit (no silent `let _ =`, no bare `unwrap()`)
- [ ] Async code doesn't block the runtime
- [ ] No race conditions in shared state

### Security (Zeus-specific)
- [ ] No hardcoded secrets or API keys
- [ ] Shell commands validated through zeus-aegis
- [ ] URL fetches validated through zeus-aegis allowlist
- [ ] No sensitive data in log output
- [ ] Input from API boundaries validated before use

### Code Quality
- [ ] `cargo clippy --workspace -- -D warnings` would pass
- [ ] `cargo fmt --check` would pass
- [ ] Functions are focused, files are <800 lines
- [ ] No unnecessary `#[allow(...)]` without comment
- [ ] Struct initialization: all fields accounted for (no missing field compile error)

### Tests
- [ ] New code has tests (`#[test]` or `#[tokio::test]`)
- [ ] Tests are hermetic (use `tempfile`, not live `~/.zeus/`)
- [ ] Env-gated tests use early-return pattern, not `#[cfg]`
- [ ] Existing tests still pass

### Zeus Architecture
- [ ] New crate added to workspace members + workspace.dependencies?
- [ ] Config-first: no hardcoded service URLs or paths?
- [ ] Upsert pattern used for named-entity collections?
- [ ] Gate protocol satisfied (4/4 features, 2/2 housekeeping)?

## Severity Levels

| Level | Action |
|-------|--------|
| CRITICAL | Block merge. Fix before any other work. |
| HIGH | Block merge. Must be addressed. |
| MEDIUM | Address before merge if possible; document if deferred. |
| LOW | Suggestions; non-blocking. |
| NIT | Style; up to author. |

## Gate Review Format

```
GATE N/N `branch-name` `commit` ✅/❌ LGTM/BLOCKED

- Finding 1 [CRITICAL/HIGH/etc]: ...
- Finding 2: ...

Verified: cargo clippy ✅ | cargo test ✅ | cargo fmt ✅
```
