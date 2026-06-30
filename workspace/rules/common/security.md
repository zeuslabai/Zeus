# Security Guidelines (Zeus / Rust)

## Mandatory Checks Before Every Commit

- [ ] No hardcoded secrets (API keys, passwords, tokens) — use env vars or `~/.zeus/.env`
- [ ] All user inputs validated at API boundaries
- [ ] Shell commands filtered through zeus-aegis before execution
- [ ] URL fetches validated against allowlist (zeus-aegis)
- [ ] Error messages don't leak sensitive data (session IDs, keys, paths)
- [ ] No `unwrap()` on external/user-provided data paths

## Secret Management

- NEVER hardcode secrets in source code or CLAUDE.md
- ALWAYS use environment variables (see `~/.zeus/.env`)
- Validate required secrets are present at startup (`zeus doctor`)
- Rotate any secret that may have been exposed immediately

## Zeus-Specific Security Patterns

- Shell tool commands: must pass `aegis.check_command()` before execution
- Web fetch URLs: must pass `aegis.check_url()` before fetch
- Sensitive ops (file delete, system config): create pending approval via `aegis.request_approval()`
- Audit logging: all tool executions logged by Athena

## Security Response Protocol

If a security issue is found:
1. **STOP immediately** — do not continue feature work
2. Use **security-reviewer** agent
3. Fix CRITICAL issues before any other work
4. Rotate any exposed secrets
5. Review entire affected crate for similar patterns
6. Post to Discord fleet channel with severity + fix

## Common Patterns to Flag

| Pattern | Severity | Fix |
|---------|----------|-----|
| Hardcoded API key/token | CRITICAL | Use `std::env::var()` |
| Shell command built from user input | CRITICAL | Use zeus-aegis allowlist |
| `unwrap()` on network/file/external data | HIGH | Propagate error with `?` |
| URL fetch without allowlist check | HIGH | Route through zeus-aegis |
| Sensitive data in log output | MEDIUM | Redact before logging |
| `expect()` on fallible path in prod | MEDIUM | Handle error explicitly |
