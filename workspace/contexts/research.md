# Research Context

Use this context when investigating issues, auditing code, or exploring the codebase before implementation.

## Active Mode: Research

You are in research mode. Goals:
- Understand before acting
- Gather facts, do not modify code
- Produce a clear finding report

## Research Protocol

1. **Read broadly first** — understand the full picture before diving deep
2. **Use parallel reads** — read multiple related files simultaneously
3. **Map dependencies** — trace call chains across crates
4. **Document findings** — write clear notes, post to Discord #private

## Zeus Codebase Navigation

```bash
# Find a type/struct
grep -rn "struct TypeName" crates/

# Find all callers of a function
grep -rn "fn_name(" crates/

# Find trait implementations
grep -rn "impl TraitName" crates/

# Find all API endpoints
grep -rn "\.route(" crates/zeus-api/

# Find test coverage
grep -rn "#\[test\]\|#\[tokio::test\]" crates/<crate>/
```

## Research Output Format

```
## Finding: <title>

**Location**: `crates/<crate>/src/<file>.rs:<line>`
**Summary**: <1-2 sentences>
**Details**: <full analysis>
**Impact**: <what this means>
**Recommendation**: <what to do>
```

## Common Research Tasks

- **Bug investigation**: find root cause, trace call path, identify fix scope
- **Security audit**: check all user-input paths, shell calls, URL fetches, secret handling
- **Performance audit**: find hot paths, blocking calls, N+1 queries
- **Compatibility check**: verify cross-crate struct initializers, trait implementations
