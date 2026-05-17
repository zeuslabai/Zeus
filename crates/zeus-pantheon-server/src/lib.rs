//! zeus-pantheon-server — IRC-style WebSocket collaboration server.
//!
//! # Usage
//! ```rust,ignore
//! use zeus_pantheon_server::{PantheonServer, config::PantheonServerConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = PantheonServerConfig::default();
//!     PantheonServer::new(config).serve().await.unwrap();
//! }
//! ```

pub mod auth;
pub mod channels;
pub mod client;
pub mod config;
pub mod messages;
pub mod protocol;
pub mod rate_limiter;
pub mod server;
pub mod state;
pub mod users;

use config::PantheonServerConfig;
use state::ServerState;

pub struct PantheonServer {
    config: PantheonServerConfig,
}

impl PantheonServer {
    pub fn new(config: PantheonServerConfig) -> Self {
        Self { config }
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        let state = ServerState::new(&self.config.default_channels, self.config.history_limit);
        server::run(self.config, state).await
    }
}
