# Configuration

Zeus is configured through three mechanisms:

1. **Config file** (`~/.zeus/config.toml`) -- The primary configuration file containing model selection, subsystem settings, and channel credentials. See [config.toml Reference](config-toml.md).

2. **Environment variables** -- API keys and credentials for LLM providers and external services. These are never stored in the config file. See [Environment Variables](environment-variables.md).

3. **Workspace files** -- Markdown files that define the agent's system prompt, personality, user context, and memory. See [Workspace Files](workspace-files.md).

## Quick Start

The fastest way to configure Zeus is with the onboarding wizard:

```bash
zeus onboard
```

This walks through provider selection, API key setup, and workspace initialization.

## Viewing Configuration

```bash
zeus config                 # Show current config (secrets redacted)
zeus config --show-secrets  # Show config with API keys visible
zeus doctor                 # Validate config and check for issues
```

## Config File Location

The config file is located at `~/.zeus/config.toml`. Zeus creates this file with defaults on first run if it does not exist. All fields are optional -- Zeus uses sensible defaults for anything not specified.
