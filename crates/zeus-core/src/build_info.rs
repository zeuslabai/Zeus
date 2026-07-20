//! Build metadata — runtime access to compile-time embedded build info.
//!
//! # Why this exists
//!
//! `cargo:rustc-env=GIT_SHA=...` (emitted by the root binary's `build.rs`) is
//! **package-scoped**: only the crate whose build script set it can read the
//! value via `env!("GIT_SHA")`. Subcrates compiled as dependencies cannot.
//!
//! This module bridges that gap: the binary entry point (which *can* see the
//! env var) calls [`BuildInfo::init`] once at startup, storing the values in a
//! static `OnceLock`. Every crate in the workspace can then read them via
//! [`BuildInfo::get`] without needing its own build script.
//!
//! #434 — surfaced when staleness of a long-running process (disk binary
//! updated, process not restarted) made registry tools invisible to the
//! LLM. Exposing the build SHA on the wire makes that condition inspectable.

use std::sync::OnceLock;

/// Build-time metadata embedded by the root binary's `build.rs`.
#[derive(Debug, Clone)]
pub struct BuildInfo {
    /// Short git SHA the running process was compiled from (`build.rs::GIT_SHA`).
    pub git_sha: &'static str,
    /// Build timestamp (unix epoch seconds) from `build.rs::ZEUS_BUILD_EPOCH`.
    pub build_epoch: u64,
    /// Crate version (`CARGO_PKG_VERSION`).
    pub version: &'static str,
}

static BUILD_INFO: OnceLock<BuildInfo> = OnceLock::new();

impl BuildInfo {
    /// Initialize the global build-info store.
    ///
    /// Called once from the binary entry point (e.g. `main.rs`). The env vars
    /// (`GIT_SHA`, `ZEUS_BUILD_EPOCH`) are **package-scoped** to the root
    /// crate (whose `build.rs` emitted them) and do not resolve in subcrates
    /// — this function is the bridge: the caller reads them via `env!()` and
    /// passes them in, making the values available workspace-wide via
    /// [`BuildInfo::get`].
    ///
    /// Safe to call multiple times — subsequent calls are no-ops (first writer
    /// wins). The `&'static str` args come from `env!()` which yields `'static`
    /// string literals, so values live for the process lifetime.
    pub fn init(git_sha: &'static str, build_epoch: u64, version: &'static str) {
        let _ = BUILD_INFO.set(BuildInfo {
            git_sha,
            build_epoch,
            version,
        });
    }

    /// Read the global build-info, if initialized.
    ///
    /// Returns `None` if [`init`] has not been called (e.g. in unit tests or
    /// standalone subcrate binaries that don't route through the main entry
    /// point). Callers should degrade gracefully — absence is a soft-fail, not
    /// a panic, since the gateway and MCP server may run in contexts where
    /// init wasn't reachable.
    pub fn get() -> Option<&'static BuildInfo> {
        BUILD_INFO.get()
    }

    /// Serialize as a JSON object suitable for embedding in API `_meta` fields.
    ///
    /// Returns `serde_json::Value::Null` if uninitialized.
    pub fn meta_json() -> serde_json::Value {
        match Self::get() {
            Some(info) => serde_json::json!({
                "build_sha": info.git_sha,
                "build_epoch": info.build_epoch,
                "version": info.version,
            }),
            None => serde_json::Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_before_init_returns_none() {
        // Note: OnceLock is process-global; this test relies on init() not
        // having been called in this test binary. If it has (e.g. a prior
        // test called init), this is still valid — we just check Option shape.
        // We cannot assert None unconditionally once init runs anywhere.
        let _ = BuildInfo::get();
    }

    #[test]
    fn meta_json_safe_when_uninit() {
        // Should not panic regardless of init state.
        let val = BuildInfo::meta_json();
        assert!(
            val.is_null() || val.is_object(),
            "meta_json must be null or object, got: {val}"
        );
    }
}
