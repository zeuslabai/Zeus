# Aegis Sandbox

The `zeus-aegis` crate provides platform-specific sandboxing that restricts what the Zeus process can do at the operating system level. It also includes command filtering and URL allowlisting that operate at the application level.

## Sandbox Levels

Five levels are available, configured via `sandbox_level` in the `[aegis]` section of `config.toml`:

### None

No restrictions. Intended for development only. The agent can access any file, connect to any host, and execute any system call.

### Basic

Blocks dangerous system operations. On macOS, this uses Seatbelt profiles to deny `ptrace`, `kexec`, and other privileged syscalls. On Linux, seccomp-bpf filters block the equivalent syscall numbers. Filesystem and network access remain unrestricted.

### Standard (Default)

Restricts filesystem access. The agent can only read and write within the workspace directory (`~/.zeus/workspace/`) and any paths explicitly added via `sandbox.allow_path()`. Attempts to access other paths are denied. Network access is unrestricted.

### Strict

Adds network restrictions on top of Standard. Only hosts listed in the `network_allowlist` configuration can be contacted. This is enforced by the `NetworkFilter`, which checks every outgoing connection against the allowlist. Useful for production deployments where you want to limit the agent to known API endpoints.

### Paranoid

The most restrictive level. Minimal permissions: only the workspace directory for filesystem access, only explicitly allowed hosts for network access, and the tightest syscall filter. Suitable for high-security environments.

## macOS Seatbelt

On macOS, Aegis uses `sandbox-exec` with Seatbelt profiles written in SBPL (Sandbox Profile Language). The profile is generated dynamically based on the configured level and allowed paths/hosts. Key restrictions include:

- **File access**: Read-only for system paths, read-write only for the workspace and allowed paths.
- **Network**: At Strict level and above, only allowed hosts can be resolved and connected to.
- **Process**: Blocks process tracing, debugging, and spawning of restricted executables.
- **IPC**: Limits inter-process communication to necessary services.

## Linux seccomp

On Linux, Aegis uses seccomp-bpf (Berkeley Packet Filter) for syscall filtering. The filter is applied to the current process and inherited by child processes. Denied syscalls return `EPERM` rather than killing the process, allowing graceful error handling.

## Command Filtering

Independent of OS-level sandboxing, Aegis filters shell commands before execution. The `ApprovalManager` checks every `shell` tool invocation against a list of dangerous patterns:

- `rm -rf` / `rm -r`
- `sudo`
- `DROP TABLE` / `DROP DATABASE`
- `chmod 777`
- `mkfs`
- `dd if=`
- Custom patterns configured by the user

Commands matching these patterns are held for approval rather than executed immediately (see [Permissions & Approvals](./permissions.md)).

## URL Allowlisting

The `NetworkFilter` validates URLs before `web_fetch` executes. At `Standard` level and below, all URLs are allowed. At `Strict` level and above, only URLs whose host matches the `network_allowlist` are permitted.

The default allowlist (when using Strict mode) includes common LLM API endpoints:

```
api.anthropic.com
api.openai.com
generativelanguage.googleapis.com
api.groq.com
api.mistral.ai
api.together.xyz
api.fireworks.ai
```

Additional hosts can be added via configuration:

```toml
[aegis]
sandbox_level = "strict"
network_allowlist = [
    "api.anthropic.com",
    "api.openai.com",
    "custom-api.example.com",
]
```

## Path Restrictions

When the sandbox level is `Standard` or higher, filesystem access is restricted. The `Sandbox` struct maintains a list of allowed paths and provides an `is_path_allowed()` check that the agent loop calls before any file operation.

Allowed paths always include:
- The workspace directory (`~/.zeus/workspace/`)
- The Zeus configuration directory (`~/.zeus/`)
- System temporary directories (`/tmp`)

Additional paths can be allowed via `sandbox.allow_path()` or through the `permissions` config list.

## Configuration

```toml
[aegis]
# Sandbox level: none, basic, standard, strict, paranoid
sandbox_level = "standard"

# Keychain service name for secret storage
keychain_service = "zeus"

# Audit log file path
audit_path = "~/.zeus/audit.log"

# Allowed operations (e.g., specific tool names)
permissions = []

# Network allowlist (hosts allowed at strict+ levels)
network_allowlist = ["api.anthropic.com", "api.openai.com"]
```
