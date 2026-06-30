//! Per-cook execution context (#192b, P1).
//!
//! `CookContext` is the per-cook carrier that decouples session ownership from
//! the shared `Agent`. Today the gateway cook loops swap the shared agent's
//! `session` field in-place per message (`set_session`), which forces an
//! `agent.write()` on every cook — the "inner lane" that re-serializes all
//! cooks fleet-wide regardless of session key.
//!
//! P1 routes the per-conversation `Session` through a `CookContext` instead, so
//! a cook never mutates shared agent state. Everything else the cook shares
//! (LLM client, tools, memory, subagents) stays on the single shared `Agent`,
//! read via `agent.read()`.
//!
//! P1 keeps the struct minimal — `session` only. P2's dispatcher (per-session
//! lanes) adds its own fields here so cook signatures are threaded once now and
//! reused across phases (clean seam, no re-threading).

use zeus_session::Session;

/// Per-cook execution context. Carries the conversation `Session` that a single
/// cook runs against, decoupled from the shared `Agent`'s session field.
///
/// Resolved from + persisted back to the shared session store (#192 store stays
/// intact); only the in-`Agent` field-swap goes away.
// Wired into the gateway cook loops in the P1 loop-threading sub-cook; the seam
// lands first so cook signatures thread once (reused by P2's dispatcher).
#[allow(dead_code)]
pub struct CookContext {
    session: Session,
}

impl CookContext {
    /// Construct a per-cook context around an already-resolved `Session`.
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Borrow the cook's session (read-only access during execution).
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Mutably borrow the cook's session (history build / persist).
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Consume the context, returning the owned `Session` for persistence back
    /// to the shared store.
    pub fn into_session(self) -> Session {
        self.session
    }
}
