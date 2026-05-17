# Command Reference

This page lists every Zeus CLI command with its arguments and behavior.

## TUI

```bash
zeus                    # Launch TUI (default when no subcommand given)
zeus tui                # Launch TUI explicitly
```

The TUI provides 10 screens: Chat, Tools, Memory, Agents, Status, Help, Settings, Teams, Extensions, and Sandbox. Navigate between screens using the keybindings shown on the Help screen.

## API Server

```bash
zeus serve              # Start the HTTP API server on the default port
zeus serve -p 3000      # Start on a custom port
```

The API server exposes 95 REST endpoints and a WebSocket endpoint for streaming. See the [API Reference](../api-reference/README.md) for full endpoint documentation.

## Gateway

```bash
zeus gateway                              # Run unified daemon (API + channels + heartbeat + cron)
zeus gateway -H 0.0.0.0 -p 3001          # Bind to all interfaces on port 3001
zeus gateway --no-channels               # Disable messaging channel adapters
zeus gateway --no-cron                   # Disable cron scheduler
zeus gateway --no-heartbeat              # Disable Prometheus heartbeat loop
zeus gateway --no-mcp                    # Disable embedded MCP server
zeus gateway --mcp-port 3002             # Set MCP server port (default: 3002)
zeus gateway --connect-hub ws://192.168.1.112:8080/v1/ws/nodes  # Join a hub as a fleet node
zeus gateway --no-channels --no-cron     # Minimal gateway (API only)
```

The gateway is the recommended way to run Zeus as a background service. It combines the API server with messaging channel adapters, the Prometheus heartbeat for proactive tasks, and the cron scheduler for recurring jobs. The `--connect-hub` flag registers this instance as a fleet node, enabling hub-spoke multi-agent topologies.

## Chat

```bash
zeus chat "Hello"       # Send a single message and print the response
zeus chat -s "Hello"    # Send a single message with streaming output
```

Chat mode is useful for scripting and quick interactions. The response is printed to stdout, making it easy to pipe into other commands.

## Tool Execution

```bash
zeus tool list_dir '{"path":"."}'    # Execute a tool directly with JSON arguments
```

Run any of the 212 available tools from the command line. The argument is a JSON object matching the tool's input schema. Output is printed to stdout.

## Configuration

```bash
zeus config                 # Show current configuration (secrets redacted)
zeus config --show-secrets  # Show configuration including API keys
```

Displays the active configuration from `~/.zeus/config.toml` along with resolved defaults. Use `--show-secrets` to include API keys and tokens in the output.

## Memory

```bash
zeus memory show               # Show current workspace context
zeus memory remember "fact"    # Add a fact to MEMORY.md (long-term memory)
zeus memory note "content"     # Add content to today's daily note
```

Memory commands interact with the workspace files in `~/.zeus/workspace/`. Facts added with `remember` persist in `memory/MEMORY.md`. Notes added with `note` go to `daily/YYYY-MM-DD.md`.

## Sessions

```bash
zeus session list              # List all sessions
zeus session show <id>         # Show messages from a session
zeus session export <id> out.md  # Export a session to markdown
```

Sessions are stored as JSONL files in the sessions directory (default `~/.zeus/sessions/`). Each session captures the full conversation history including tool calls and results.

## Diagnostics

```bash
zeus doctor                    # Run system diagnostics
```

The doctor command checks configuration validity, workspace structure, API key presence, Ollama connectivity, and other system health indicators. Run this first when troubleshooting issues.

## Onboarding

```bash
zeus onboard                   # Run the interactive setup wizard
```

The onboarding wizard walks through initial configuration: choosing an LLM provider, setting API keys, and creating workspace files. Recommended for first-time setup.

## Daemon Management

```bash
zeus daemon install            # Install launchd service (macOS)
zeus daemon start              # Start the daemon
zeus daemon stop               # Stop the daemon
zeus daemon status             # Check daemon status
```

The daemon commands manage Zeus as a macOS launchd service. After `install`, the daemon starts automatically on login and runs the gateway in the background.

## Shell Completions

```bash
zeus completion bash           # Generate bash completions
zeus completion zsh            # Generate zsh completions
zeus completion fish           # Generate fish completions
```

Generate shell completion scripts for tab-completion of Zeus commands and arguments. See the [Completions](completions.md) page for installation instructions.
