# First Run

Before launching Zeus, make sure you have at least one LLM provider API key set as an environment variable. For example, to use Anthropic:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

See [Environment Variables](../configuration/environment-variables.md) for the full list of supported providers.

## Interactive Setup Wizard

The easiest way to get started is the onboarding wizard:

```bash
zeus onboard
```

This walks you through selecting an LLM provider, entering your API key, and creating the initial workspace files at `~/.zeus/workspace/`.

## Launch the TUI

To open the terminal user interface:

```bash
zeus
```

or explicitly:

```bash
zeus tui
```

The TUI presents a chat screen where you can type messages, view streaming responses, browse tools, inspect memory, and manage settings across 10 screens.

## Single Message Mode

For quick, one-off interactions without entering the TUI:

```bash
zeus chat "What is the capital of France?"
```

Add the `-s` flag to enable streaming output:

```bash
zeus chat -s "Summarize the files in my workspace"
```

## Run Diagnostics

To verify that your configuration, workspace, credentials, and optional services are set up correctly:

```bash
zeus doctor
```

The doctor command checks:

- Config file validity (`~/.zeus/config.toml`)
- Workspace directory and required files
- API key presence for the configured provider
- Ollama connectivity (if configured)

## Start the API Server

To expose Zeus as an HTTP API (useful for the iOS app, webhooks, and external integrations):

```bash
zeus serve
```

By default the server listens on port 8080. To use a different port:

```bash
zeus serve -p 3000
```

## Start the Unified Gateway

For production use, the gateway combines the API server with channel adapters, the heartbeat loop, and the cron scheduler:

```bash
zeus gateway
```

By default it binds to `127.0.0.1:8080`. To accept external connections (e.g. for the iOS app or fleet nodes):

```bash
zeus gateway -H 0.0.0.0 -p 3001
```

You can selectively disable subsystems:

```bash
zeus gateway --no-channels    # skip messaging adapters (Telegram, Discord, etc.)
zeus gateway --no-cron        # skip scheduled/recurring tasks
zeus gateway --no-heartbeat   # skip Prometheus proactive heartbeat
zeus gateway --no-mcp         # skip embedded MCP server (default port 3002)
```

To join a hub gateway as a fleet node:

```bash
zeus gateway --connect-hub ws://192.168.1.112:8080/v1/ws/nodes
```

## What to Try Next

- Type a message in the TUI and watch Zeus respond with streaming output.
- Ask Zeus to read or write files in your workspace.
- Run `zeus tool list_dir '{"path":"."}'` to execute a tool directly from the command line.
- Explore the [Configuration](./configuration.md) chapter to tune your setup.
