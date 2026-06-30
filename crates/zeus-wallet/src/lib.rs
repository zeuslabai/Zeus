//! Zeus Wallet — Ed25519 keypair wallet with x402 payment protocol
//!
//! Provides:
//! - Ed25519 keypair generation (ed25519-dalek)
//! - Message signing and verification
//! - Key persistence to ~/.zeus/wallet/
//! - x402 payment client: on HTTP 402, auto-sign token transfer and retry

pub mod keypair;
pub mod x402;

pub use keypair::{WalletError, WalletKeypair};
pub use x402::{PaymentReceipt, X402Client, X402Config};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Wallet configuration for config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Directory to store wallet keys (default: ~/.zeus/wallet/)
    #[serde(default = "default_wallet_dir")]
    pub wallet_dir: PathBuf,

    /// Enable x402 automatic payment protocol
    #[serde(default)]
    pub enable_x402: bool,

    /// Maximum amount (in micro-units, 1 token = 1_000_000) per single x402 payment
    #[serde(default = "default_max_payment")]
    pub max_payment_amount: u64,

    /// x402 payment network: "solana-devnet" or "solana-mainnet"
    #[serde(default = "default_network")]
    pub network: String,

    /// Token symbol → mint address mapping (e.g. "ZEUS" → "<mint>").
    /// Replace placeholder addresses with real mint addresses after token launch.
    #[serde(default = "default_token_mints")]
    pub token_mints: HashMap<String, String>,
}

fn default_wallet_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
        .join("wallet")
}

fn default_max_payment() -> u64 {
    1_000_000 // 1 token
}

fn default_network() -> String {
    "solana-devnet".to_string()
}

fn default_token_mints() -> HashMap<String, String> {
    // TODO: Replace with real ZEUS token mint address after token launch
    HashMap::from([("ZEUS".to_string(), "ZEUS_MINT_PLACEHOLDER".to_string())])
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            wallet_dir: default_wallet_dir(),
            enable_x402: false,
            max_payment_amount: default_max_payment(),
            network: default_network(),
            token_mints: default_token_mints(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = WalletConfig::default();
        assert!(!cfg.enable_x402);
        assert_eq!(cfg.max_payment_amount, 1_000_000);
        assert_eq!(cfg.network, "solana-devnet");
        assert!(cfg.wallet_dir.ends_with("wallet"));
        assert!(cfg.token_mints.contains_key("ZEUS"));
        assert!(!cfg.token_mints["ZEUS"].is_empty());
    }

    #[test]
    fn test_config_serialization() {
        let cfg = WalletConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let decoded: WalletConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.network, cfg.network);
        assert_eq!(decoded.enable_x402, cfg.enable_x402);
        assert_eq!(decoded.token_mints, cfg.token_mints);
    }

    #[test]
    fn test_token_mints_configurable() {
        let mut cfg = WalletConfig::default();
        cfg.token_mints.insert(
            "USDC".to_string(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
        );
        assert_eq!(cfg.token_mints.len(), 2);
        assert!(cfg.token_mints.contains_key("ZEUS"));
        assert!(cfg.token_mints.contains_key("USDC"));
    }
}
