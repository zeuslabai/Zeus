//! Standalone `zeus-tui` binary — launches the TUI (onboarding + production)
//! directly with seed state and no gateway wiring. Useful for previewing the
//! interface design in isolation.
//!
//! The integrated entrypoint used by the root `zeus` binary (`zeus` / `zeus tui`)
//! is [`zeus_tui::run`] in `lib.rs`, which wires the same `App` to the gateway.

fn main() -> std::io::Result<()> {
    zeus_tui::app::run_standalone()
}
