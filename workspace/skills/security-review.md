---
name: security-review
description: Security review for Zeus (Rust). Use when adding API endpoints, shell tool handling, web fetch, authentication, or any code touching user input or secrets. Provides OWASP-adapted checklist for Rust/Zeus.
origin: ECC (adapted for Zeus/Rust + zeus-aegis)
---

# Security Review Skill (Zeus / Rust)

## When to Activate

- Adding new API endpoints to zeus-api
- Modifying shell command execution (zeus-agent tools)
- Adding web fetch functionality
- Handling user-provided file paths
- Touching authentication or session management
- Adding new channel adapters (user messages = external input)
- Any code that reads secrets from env vars

## Security Checklist

### 1. Secrets Management

```rust
// ❌ NEVER
const API_KEY: &str = "sk-proj-xxxxx";

// ✅ ALWAYS
let api_key = std::env::var("ANTHROPIC_API_KEY")
    .map_err(|_| Error::Config("ANTHROPIC_API_KEY not set".into()))?;
```

- [ ] No hardcoded API keys, tokens, or passwords in source
- [ ] All secrets via `std::env::var()` or config file
- [ ] `zeus doctor` validates required secrets at startup
- [ ] No secrets in log output (use `[REDACTED]` pattern)

### 2. Shell Command Validation (zeus-aegis)

All shell commands MUST pass through zeus-aegis before execution:

```rust
// ✅ REQUIRED pattern for shell tool
aegis.check_command(&command).await?;
// Only execute if check passes
```

- [ ] Shell commands validated through `aegis.check_command()`
- [ ] No string interpolation of user input into shell commands
- [ ] Use `std::process::Command` with explicit args (not shell string)

### 3. URL Validation (zeus-aegis)

```rust
// ✅ REQUIRED pattern for web_fetch
aegis.check_url(&url).await?;
```

- [ ] All web fetch URLs checked against zeus-aegis allowlist
- [ ] No SSRF: user-provided URLs must be validated
- [ ] Redirect policy: use `redirect::Policy::none()` + manual re-POST for Ollama (see `36bf0f48`)

### 4. Input Validation at API Boundaries

```rust
// ✅ Validate before processing
#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    // serde handles type validation; add custom validators for business rules
}

// Validate string length, content, etc.
if request.message.is_empty() || request.message.len() > 100_000 {
    return Err(StatusCode::BAD_REQUEST);
}
```

- [ ] All API handler inputs validated (type + bounds + content)
- [ ] File paths sanitized (no `..` traversal)
- [ ] No `unwrap()` on user-provided data

### 5. Error Handling (no data leakage)

```rust
// ❌ WRONG — leaks internal details
Err(e) => return Json(json!({"error": e.to_string()})),

// ✅ CORRECT — log details, return generic message
Err(e) => {
    error!("internal error: {e}");
    return (StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "internal error"}))).into_response();
}
```

- [ ] Error messages to users are generic
- [ ] Detailed errors logged server-side only
- [ ] No stack traces, file paths, or internal state in API responses

### 6. Approval System for Sensitive Ops

High-risk operations must create a pending approval:
```rust
aegis.request_approval(&operation_description).await?;
```

- [ ] Sensitive ops (file delete, system config, mass data access) require approval
- [ ] Approvals visible via `GET /v1/approvals`

## Common Patterns to Flag

| Pattern | Severity | Fix |
|---------|----------|-----|
| Hardcoded secret in source | CRITICAL | Use `std::env::var()` |
| Shell command built from user string | CRITICAL | Use aegis + `Command::new()` with args |
| `unwrap()` on user-provided data | HIGH | Propagate with `?` |
| URL fetch without aegis check | HIGH | Add `aegis.check_url()` |
| Error message leaks internals | MEDIUM | Generic user message + server log |
| Missing rate limiting on endpoint | MEDIUM | Add rate limit middleware |
| Sensitive data in tracing span | MEDIUM | Remove or redact |

## Emergency Response

CRITICAL vulnerability found:
1. STOP all other work immediately
2. Post to Discord #private: severity + location + impact
3. Fix before any other commits
4. If secrets exposed: rotate immediately
5. Post resolution with commit hash
