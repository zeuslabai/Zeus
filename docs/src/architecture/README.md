# Architecture Overview

Zeus is a Cargo workspace comprising 20 crates and roughly 59,400 lines of Rust (plus ~2,300 lines of Swift for native apps). The design follows a hub-and-spoke model: a central **agent loop** sits at the core, with every advanced subsystem wired in as an optional layer around it.

## Design Principles

- **Single agent loop.** All user interactions -- whether from the TUI, REST API, CLI, macOS Desktop app, or iOS client -- converge into the same `Agent::run()` loop. There is one code path for message processing, not five.
- **Subsystems are opt-in.** Each advanced capability (cognitive engine, security sandbox, memory search, documentation logging, notifications) is initialized in `Agent::with_subsystems()` and injected as a trait object or `Option`. If a subsystem is not configured, the agent loop skips it. The core agent compiles and runs with zero subsystems enabled.
- **Orchestration sits above the agent.** The Prometheus crate does not live inside the agent loop. It wraps the agent: simple messages go directly to `agent.run()`, while complex tasks are decomposed by the planner and executed through the cooking loop, which repeatedly calls back into the agent.
- **Tools are the interface.** The LLM interacts with the outside world exclusively through tool calls. The 8 core tools (read_file, write_file, edit_file, list_dir, shell, web_fetch, spawn, message) cover general-purpose work. Talos adds 193 macOS automation tools. The browser crate adds 11 CDP tools. Channel adapters expose messaging platforms through the `message` tool.

## Workspace Layout

```
Zeus/
├── Cargo.toml              # Workspace root
├── crates/                 # 20 library/binary crates
│   ├── zeus-core/          # Shared types, errors, config
│   ├── zeus-agent/         # Agent loop (the hub)
│   ├── zeus-llm/           # LLM provider abstraction
│   └── ...                 # 17 more crates (see Crate Map)
├── apps/
│   ├── ZeusDesktop/        # SwiftUI macOS app (UniFFI bindings)
│   └── ZeusMobile/         # SwiftUI iOS app (REST + WebSocket)
├── docs/                   # This mdBook documentation
└── scripts/                # Build and setup scripts
```

## How the Pieces Fit Together

The dependency graph fans outward from `zeus-core` (which every crate depends on) through `zeus-llm` and `zeus-agent` up to the frontends:

```
                        ┌─────────────┐
                        │  Frontends  │
                        │ TUI / API / │
                        │ Desktop/iOS │
                        └──────┬──────┘
                               │
                        ┌──────▼──────┐
                        │  Prometheus  │  orchestration wrapper
                        │  (optional)  │
                        └──────┬──────┘
                               │
                        ┌──────▼──────┐
                  ┌─────┤    Agent    ├─────┐
                  │     │   (hub)     │     │
                  │     └──────┬──────┘     │
                  │            │            │
           ┌──────▼──┐  ┌─────▼────┐  ┌────▼─────┐
           │  Nous   │  │   LLM    │  │  Aegis   │
           │ Athena  │  │ Provider │  │ Security │
           │Mnemosyne│  │          │  │          │
           │ Hermes  │  │          │  │          │
           └─────────┘  └──────────┘  └──────────┘
                  │            │            │
                  └────────────┼────────────┘
                               │
                        ┌──────▼──────┐
                        │  zeus-core  │
                        │  (types,    │
                        │   config)   │
                        └─────────────┘
```

For the full list of crates with line counts and descriptions, see the [Crate Map](./crate-map.md). For the step-by-step message processing pipeline, see [Data Flow](./data-flow.md). For details on each subsystem's integration point, see [Subsystems](./subsystems.md).
