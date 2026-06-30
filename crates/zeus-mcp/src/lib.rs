//! Zeus MCP Server & Client - Model Context Protocol implementation
//!
//! This crate provides:
//! - An MCP **server** that exposes Zeus tools via JSON-RPC
//! - An MCP **client** that connects to external MCP servers via stdio
//! - A **tool registry** for managing tools from multiple MCP connections
//!
//! Can be used as:
//! - A library integrated into other applications
//! - A standalone server binary
//! - Optionally integrated into the main zeus binary

pub mod agents;
pub mod client;
mod handlers;
mod protocol;
mod server;
pub mod tool_registry;

pub use agents::McpAgentManager;
pub use client::{McpClient, McpClientConfig, ToolCallResult};
pub use handlers::ToolHandler;
pub use protocol::{McpError, McpMethod, McpRequest, McpResponse, ToolDefinition};
pub use server::{McpConfig, McpServer, McpStdio};
pub use tool_registry::{
    ConflictResult, ConflictStrategy, HealthMetrics, HealthStatus, RegistryError, RegistryStats,
    ServerInfo, ToolEntry, ToolRegistry,
};

/// MCP Server version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default MCP port
pub const DEFAULT_PORT: u16 = 3002;
