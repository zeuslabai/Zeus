//! Zeus Agent - Agent loop, tools, and subagents
//!
//! Connects all Zeus subsystems into a unified agent runtime.

mod agent_loop;
pub mod auth_rotation;
pub mod channel_builder;
mod channels;
pub mod concurrency;
pub mod constitution;
pub mod cook;
pub mod hooks;
pub mod intelligence;
pub mod loop_guard;
pub mod message_store_filter;
pub mod metrics;
pub mod migration;
pub mod overflow;
pub mod research;
pub mod router;
mod subagent;
pub mod survival;
pub mod token_alert;
pub mod tools;

pub use agent_loop::{Agent, AgentEvent};
pub use auth_rotation::{AuthProfile, AuthRotationManager, ProfileStatus, RotationStrategy};
pub use channels::{Channel, ChannelNotificationSender, register_channel_senders};
pub use concurrency::{ConcurrencyConfig, ConcurrencyLimiter, QueueMode};
pub use constitution::{Constitution, ConstitutionVerdict};
pub use cook::{
    AcceptanceChecker, AcceptanceResult, CookConfig, NoopAcceptanceChecker, TaskOutcome,
    TaskPersistence, TaskRecord, cook_until_done,
};
pub use hooks::{Hook, HookAction, HookContext, HookEventType, HookRegistry};
pub use intelligence::{ContextGuard, LoopDetector};
pub use metrics::AgentMetrics;
pub use migration::{ImportResult, ImportSource, MigrationEngine};
pub use overflow::{
    ContextBudget, OverflowConfig, OverflowRecovery, OverflowStatus, RecoveryAction,
};
pub use router::{AgentProfile, AgentRouter, AgentRoutingConfig};
pub use subagent::{AgentTarget, Subagent, SubagentConfig, SubagentResult, spawn_subagent};
pub use survival::{SurvivalMonitor, SurvivalThresholds, SurvivalTier};
pub use tools::{ToolRegistry, execute_tool, execute_deep_research, send_file_to_channel};

// Re-export subsystem types for consumers
pub use zeus_talos::TalosRegistry;
