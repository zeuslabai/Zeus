//! #331 — macOS sleep immunity for the gateway.
//!
//! macOS Maintenance-Sleep/DarkWake cycles freeze a running gateway (zeus112's
//! Discord flap: every adapter reconnect matched a DarkWake in `pmset -g log`).
//! The durable fix is the gateway holding an IOPM power assertion
//! (`PreventSystemSleep`) for its own lifetime, so the OS never idle-sleeps it.
//!
//! Mechanism notes (research-verified on a live Mac seat, 2026-07-09):
//! - Raw FFI against IOKit/CoreFoundation — ZERO crate dependencies. The
//!   framework link + two extern fns is all this needs.
//! - powerd auto-releases assertions when the owning process dies — verified
//!   empirically via `std::process::exit(3)` with no release call: the
//!   assertion vanished from `pmset -g assertions` within 1s. So the #329
//!   hard-exit hatch, panics, and SIGKILL all self-clean; the explicit
//!   release on the happy path is tidiness, not load-bearing.
//! - HONEST BOUNDARY: `PreventSystemSleep` stops *idle* and maintenance
//!   sleep. It does NOT survive lid-close on laptops (that requires
//!   `pmset disablesleep 1`, root) and may be ignored on battery power.
//!   Desktop seats (Studio / mini) get full coverage.
//!
//! Non-macOS: everything here is a structural no-op.

/// kIOReturnSuccess — the only IOReturn value that means the assertion holds.
pub const KIO_RETURN_SUCCESS: i32 = 0;

/// Pure decision fn: interpret `IOPMAssertionCreateWithName`'s result.
/// Returns the assertion id to retain (and later release) iff the call
/// succeeded. Kept pure so the gate can sabotage-test the decision logic
/// without touching powerd.
pub fn assertion_id_from_result(ret: i32, id: u32) -> Option<u32> {
    if ret == KIO_RETURN_SUCCESS {
        Some(id)
    } else {
        None
    }
}

/// RAII guard for a macOS IOPM `PreventSystemSleep` assertion.
///
/// Holds the assertion id when active; releases it on `Drop` (normal
/// teardown). Abnormal exits are covered by powerd's process-death cleanup
/// (see module docs).
pub struct PowerAssertion {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    id: Option<u32>,
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{c_char, c_void};

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: *const c_void,
            assertion_level: u32,
            assertion_name: *const c_void,
            assertion_id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(assertion_id: u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

    /// Create the `PreventSystemSleep` assertion. Returns the raw
    /// (IOReturn, assertion_id) pair for the pure decision fn to judge.
    pub(super) fn create_assertion() -> (i32, u32) {
        // SAFETY: both CFStrings are created from valid NUL-terminated
        // literals, passed to IOPM while alive, and released after the call.
        // `id` is a plain out-param u32.
        unsafe {
            let ty = CFStringCreateWithCString(
                std::ptr::null(),
                c"PreventSystemSleep".as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            );
            let name = CFStringCreateWithCString(
                std::ptr::null(),
                c"Zeus gateway — prevent sleep while serving (#331)".as_ptr(),
                K_CF_STRING_ENCODING_UTF8,
            );
            if ty.is_null() || name.is_null() {
                // CFString allocation failure — treat as assertion-denied.
                if !ty.is_null() {
                    CFRelease(ty);
                }
                if !name.is_null() {
                    CFRelease(name);
                }
                return (-1, 0);
            }
            let mut id: u32 = 0;
            let ret = IOPMAssertionCreateWithName(ty, K_IOPM_ASSERTION_LEVEL_ON, name, &mut id);
            CFRelease(ty);
            CFRelease(name);
            (ret, id)
        }
    }

    pub(super) fn release_assertion(id: u32) {
        // SAFETY: id came from a successful IOPMAssertionCreateWithName.
        // Releasing an already-dead id is harmless (powerd ignores it).
        unsafe {
            IOPMAssertionRelease(id);
        }
    }
}

impl PowerAssertion {
    /// Acquire sleep immunity if `prevent_sleep` is enabled. Best-effort:
    /// failure is a WARN, never fatal to gateway start. Always returns a
    /// guard (possibly inert) so call sites hold one shape.
    pub fn acquire(prevent_sleep: bool) -> Self {
        if !prevent_sleep {
            tracing::info!(
                target: "boot",
                event = "power_assertion",
                "prevent_sleep disabled by config — gateway will not block system sleep"
            );
            return Self { id: None };
        }
        Self::acquire_platform()
    }

    #[cfg(target_os = "macos")]
    fn acquire_platform() -> Self {
        let (ret, raw_id) = macos::create_assertion();
        let id = assertion_id_from_result(ret, raw_id);
        match id {
            Some(id) => {
                tracing::info!(
                    target: "boot",
                    event = "power_assertion",
                    assertion_id = id,
                    "PreventSystemSleep assertion held — macOS will not idle-sleep this gateway \
                     (boundary: does not survive lid-close; may be ignored on battery)"
                );
            }
            None => {
                tracing::warn!(
                    target: "boot",
                    event = "power_assertion",
                    ioreturn = ret,
                    "IOPMAssertionCreateWithName failed — gateway runs WITHOUT sleep immunity; \
                     operator fallback: `caffeinate -is` or `pmset sleep 0`"
                );
            }
        }
        Self { id }
    }

    #[cfg(not(target_os = "macos"))]
    fn acquire_platform() -> Self {
        // Non-macOS: sleep semantics are the service manager's problem
        // (systemd/rc.d boxes don't maintenance-sleep). Structural no-op.
        Self { id: None }
    }

    /// Explicit release for the normal teardown path. Idempotent.
    pub fn release(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(id) = self.id.take() {
            macos::release_assertion(id);
            tracing::info!(
                target: "boot",
                event = "power_assertion",
                assertion_id = id,
                "PreventSystemSleep assertion released"
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.id = None;
        }
    }
}

impl Drop for PowerAssertion {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assertion_id_only_retained_on_success() {
        // Success: keep the id powerd handed us.
        assert_eq!(assertion_id_from_result(KIO_RETURN_SUCCESS, 42), Some(42));
        // Any nonzero IOReturn (e.g. kIOReturnNotPrivileged) → no assertion.
        assert_eq!(assertion_id_from_result(-536_870_174, 42), None);
        assert_eq!(assertion_id_from_result(1, 42), None);
        assert_eq!(assertion_id_from_result(-1, 0), None);
    }

    #[test]
    fn disabled_knob_yields_inert_guard() {
        let mut guard = PowerAssertion::acquire(false);
        assert!(guard.id.is_none());
        // release on an inert guard is a no-op, not a crash
        guard.release();
        assert!(guard.id.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn acquire_and_release_against_live_powerd() {
        // Real end-to-end: create the assertion, verify we hold an id,
        // release it. powerd accepts unprivileged PreventSystemSleep, so
        // this is deterministic on any mac CI/seat.
        let mut guard = PowerAssertion::acquire(true);
        assert!(
            guard.id.is_some(),
            "IOPMAssertionCreateWithName should succeed for an unprivileged process"
        );
        guard.release();
        assert!(guard.id.is_none());
        // Double-release must be idempotent.
        guard.release();
    }
}
