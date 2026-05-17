# CLI Reference

Zeus provides a command-line interface for interacting with the AI assistant. The CLI supports multiple modes of operation:

- **TUI** -- A full terminal user interface with 10 screens for chat, tools, memory, and more.
- **API Server** -- An HTTP server exposing 95 REST endpoints and WebSocket streaming.
- **Gateway** -- A unified daemon combining the API server with channels, heartbeat, and cron scheduling.
- **Single Message** -- Send a one-off message and print the response directly to stdout.
- **Utility Commands** -- Manage configuration, memory, sessions, and system health.

Run `zeus --help` to see all available subcommands, or continue reading for the full command reference.
