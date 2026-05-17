//! Linux-specific security features
//!
//! Provides sandboxing via seccomp-bpf (Secure Computing Mode).

pub mod seccomp;

pub use seccomp::{SeccompFilter, SeccompSandbox};
