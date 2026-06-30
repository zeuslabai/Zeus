//! # zeus-plugins
//!
//! Plugin system foundation for the Zeus platform.
//!
//! Provides:
//! - [`Plugin`] — the core trait every plugin must implement
//! - [`PluginRegistry`] — runtime registry for managing plugins
//! - [`loader`] — directory scanner for discovering plugin manifests
//!
//! This crate is intentionally **not wired into the agent loop** — it is the
//! foundation layer only. Higher-level crates (e.g. `zeus-agent`) are
//! responsible for integration.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use zeus_plugins::{Plugin, PluginRegistry};
//! use zeus_plugins::loader::discover_plugins;
//! use std::path::Path;
//!
//! // Discover plugins in a directory
//! let discovered = discover_plugins(Path::new("/etc/zeus/plugins"))?;
//! for p in &discovered {
//!     println!("Found: {} v{}", p.manifest.name, p.manifest.version);
//! }
//!
//! // Register and use plugins
//! let mut registry = PluginRegistry::new();
//! registry.register(Box::new(my_plugin)).await?;
//! let result = registry.execute_tool("my_plugin", "do_thing", args).await?;
//! ```

pub mod loader;
pub mod plugin;
pub mod registry;

pub use loader::{DiscoveredPlugin, LoaderError, PluginManifest, discover_plugins, load_manifest};
pub use plugin::Plugin;
pub use registry::PluginRegistry;
