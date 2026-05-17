//! macOS-specific security features
//!
//! Provides sandboxing via sandbox-exec (Seatbelt profiles).

pub mod seatbelt;

pub use seatbelt::{SeatbeltProfile, SeatbeltSandbox};
