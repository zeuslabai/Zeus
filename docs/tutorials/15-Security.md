# Security — Aegis Sandbox

Zeus includes a comprehensive security system called Aegis. It provides sandboxing, command filtering, URL allowlisting, credential management, and audit logging.

## Sandbox Levels

Configure the security posture in `config.toml`:

```toml
[aegis]
sandbox_level = "standard"   # Default
```

| Level | Description |
|-------|-------------|
| `none` | No restrictions (development only) |
| `basic` | Command blocklist only |
| `standard` | Command filter + path restrictions + URL allowlist |
| `strict` | All of standard + approval required for sensitive ops |
| `paranoid` | All tools require explicit approval |

## Command Filtering

Aegis blocks dangerous commands automatically:

```bash
# These are blocked at "standard" level and above:
zeus tool shell '{"command":"rm -rf /"}'         # ❌ Blocked
zeus tool shell '{"command":"mkfs.ext4 /dev/sda"}' # ❌ Blocked
zeus tool shell '{"command":":(){ :|:& };:"}'    # ❌ Blocked (fork bomb)

# Safe commands work normally:
zeus tool shell '{"command":"ls -la"}'            # ✅ Allowed
zeus tool shell '{"command":"cargo test"}'        # ✅ Allowed
```

## URL Allowlisting

Control which URLs Zeus can access:

```toml
[aegis]
network_allowlist = ["*"]              # Allow all (default)
# network_allowlist = [
#   "https://api.github.com/*",
#   "https://httpbin.org/*",
#   "https://*.example.com/*"
# ]
```

SSRF protection is built in — Zeus blocks requests to internal IPs and metadata endpoints by default.

## Path Restrictions

At `standard` level and above, Zeus restricts file access to:
- The configured workspace directory
- The current working directory
- `/tmp/`
- Explicitly allowed paths

## Credential Vault

Zeus stores credentials securely:

1. **OS Keychain** (preferred) — macOS Keychain, Linux Secret Service
2. **Config fallback** — `~/.zeus/.env` file (chmod 600)

```bash
# All secrets belong in ~/.zeus/.env
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
DISCORD_BOT_TOKEN=...
```

> ⚠️ Never put credentials in `config.toml`, commit them to git, or post them in chat channels.

## Audit Logging

All tool executions are logged:

```toml
[aegis]
audit_path = "~/.zeus/audit.log"
```

View the audit log:

```bash
# Recent entries
tail -20 ~/.zeus/audit.log

# Via API
curl http://localhost:3001/v1/security/threats | jq
```

### Session Redaction

Sensitive data (API keys, tokens, passwords) is automatically redacted from:
- Session transcripts
- Audit logs
- Chat exports

## Permissions

Configure tool-level permissions:

```toml
[aegis]
permissions = ["*"]                    # Allow all tools (default)
# permissions = [
#   "read_file",
#   "write_file",
#   "list_dir",
#   "shell",
#   "web_fetch"
# ]
```

### Approval System

At `strict` level, sensitive operations create pending approvals:

```bash
# View pending approvals
curl http://localhost:3001/v1/approvals | jq

# Approve
curl -X POST http://localhost:3001/v1/approvals/<id>/approve

# Reject
curl -X POST http://localhost:3001/v1/approvals/<id>/reject
```

## macOS Seatbelt Sandbox

On macOS, Zeus can use Apple's Seatbelt sandboxing for process-level isolation:

```toml
[aegis]
sandbox_level = "standard"
```

This generates and applies Seatbelt profiles that restrict:
- File system access
- Network access
- Process execution
- System call access

## Security Checklist

- [ ] API keys in `~/.zeus/.env` (not in config.toml or shell profile)
- [ ] `~/.zeus/.env` permissions set to `chmod 600`
- [ ] Audit logging enabled
- [ ] Sandbox level appropriate for your use case
- [ ] Network allowlist configured (if not using `*`)
- [ ] Gateway auth token set for remote access

## What's Next

→ [[16-Deployment]] — Deploy Zeus as a system service
→ [[03-Configuration]] — Full config reference
