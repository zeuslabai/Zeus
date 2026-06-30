# Security

Zeus takes a defense-in-depth approach to security through its `zeus-aegis` crate. Every tool execution passes through multiple security checks before running, and all actions are logged in a tamper-evident audit trail.

## Security Layers

| Layer | Component | Description |
|-------|-----------|-------------|
| **Sandboxing** | Seatbelt (macOS) / seccomp (Linux) | OS-level process sandboxing that restricts syscalls, filesystem access, and network connections. |
| **Command Filtering** | `ApprovalManager` | Pattern-based detection of dangerous shell commands (`rm -rf`, `sudo`, `DROP TABLE`, etc.) with an approval workflow. |
| **URL Allowlisting** | `NetworkFilter` | Validates `web_fetch` targets against an allowlist. At `Strict` level and above, only explicitly allowed hosts can be contacted. |
| **Path Restrictions** | `Sandbox` | Restricts filesystem access to the workspace directory and explicitly allowed paths. |
| **Approval System** | `ApprovalManager` | Tools and commands that match dangerous patterns require human approval before execution. |
| **Audit Logging** | `AuditLog` + `zeus-athena` | Every tool execution, network request, file access, and permission check is logged with a tamper-evident hash chain. |
| **Keychain Integration** | `Keychain` | API keys and secrets are stored in the OS keychain (macOS Keychain, Linux Secret Service) rather than in plain text config files. |

## Security Levels

Aegis provides five security levels, from least to most restrictive:

| Level | Filesystem | Network | Description |
|-------|-----------|---------|-------------|
| `none` | Unrestricted | Unrestricted | No restrictions. Development only. |
| `basic` | Unrestricted | Unrestricted | Blocks dangerous syscalls (ptrace, kexec, etc.) |
| `standard` | Restricted | Unrestricted | Filesystem limited to workspace and allowed paths. Default level. |
| `strict` | Restricted | Allowlist | Network restricted to explicitly allowed hosts. |
| `paranoid` | Restricted | Allowlist | Minimal permissions. Most restrictive level. |

## Quick Start

Set the security level in `config.toml`:

```toml
[aegis]
sandbox_level = "standard"
network_allowlist = ["api.anthropic.com", "api.openai.com"]
```

For details on each component, see:

- [Aegis Sandbox](./aegis.md) -- Sandbox levels, Seatbelt profiles, and command filtering.
- [Permissions & Approvals](./permissions.md) -- The approval workflow for dangerous operations.
- [Audit Logging](./audit.md) -- The tamper-evident audit trail and session audit endpoints.
