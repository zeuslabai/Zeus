//! Pantheon — IRC-style agent communication network for Zeus TUI.
//!
//! Architecture: modular widget-based design
//! - app.rs         — Pantheon app state, message handling, channel management
//! - login.rs       — Login screen widget + auth flow
//! - chat.rs        — Main 3-column chat layout
//! - channel_list.rs — Channel sidebar widget
//! - message_view.rs — Message display + formatting
//! - user_list.rs    — User sidebar widget with presence
//! - input_bar.rs    — IRC-style input + command parsing
//! - commands.rs     — IRC command handlers (/me, /join, /topic, etc.)
//! - nick_color.rs   — Deterministic nick coloring

pub mod app;
pub mod client;
pub mod config;
pub mod login;
pub mod chat;
pub mod channel_list;
pub mod message_view;
pub mod user_list;
pub mod input_bar;
pub mod commands;
pub mod nick_color;
