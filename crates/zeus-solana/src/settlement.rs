//! `SolanaSettlement` — bridges the Agora marketplace to on-chain SPL transfers.

use tracing::info;
use zeus_agora::AgoraError;
use zeus_agora::{SettlementProvider, SettlementReceipt};

use crate::transfer::{SolanaTransferError, TransferParams, submit_transfer};

/// Settlement backend that submits real SPL token transfers on Solana.
///
/// Plugs into `Marketplace::with_settlement()` to replace in-memory or x402
/// settlement with actual on-chain token movement.
pub struct SolanaSettlement {
    /// Solana RPC endpoint (e.g. `https://api.devnet.solana.com`).
    pub rpc_url: String,
    /// Sender keypair bytes (64 bytes: secret || public).
    pub sender_keypair_bytes: Vec<u8>,
    /// SPL token mint address (base58).
    pub mint: String,
    /// Token decimals (e.g. 6 for USDC).
    pub decimals: u8,
    /// Credits-to-base-unit conversion rate.
    /// `base_units = credits * micro_units_per_credit`.
    /// For example, if 1 credit = 1 µUSDC, set to 1.
    pub base_units_per_credit: u64,
}

impl SolanaSettlement {
    pub fn new(
        rpc_url: impl Into<String>,
        sender_keypair_bytes: Vec<u8>,
        mint: impl Into<String>,
        decimals: u8,
        base_units_per_credit: u64,
    ) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            sender_keypair_bytes,
            mint: mint.into(),
            decimals,
            base_units_per_credit,
        }
    }

    /// Convert marketplace credits to SPL token base units.
    fn credits_to_base_units(&self, credits: i64) -> u64 {
        (credits.max(0) as u64).saturating_mul(self.base_units_per_credit)
    }
}

impl SettlementProvider for SolanaSettlement {
    fn settle(
        &self,
        _buyer_id: &str,
        seller_id: &str,
        amount_credits: i64,
        skill_name: &str,
    ) -> Result<SettlementReceipt, AgoraError> {
        let amount = self.credits_to_base_units(amount_credits);
        if amount == 0 {
            return Err(AgoraError::SettlementFailed(
                "Zero-amount transfers not allowed".to_string(),
            ));
        }

        info!(
            %seller_id,
            %skill_name,
            credits = amount_credits,
            base_units = amount,
            "Submitting on-chain SPL settlement"
        );

        let params = TransferParams {
            rpc_url: self.rpc_url.clone(),
            sender_keypair_bytes: self.sender_keypair_bytes.clone(),
            recipient: seller_id.to_string(),
            mint: self.mint.clone(),
            amount,
            decimals: self.decimals,
        };

        let result = submit_transfer(&params).map_err(|e| match e {
            SolanaTransferError::InsufficientBalance { have, need } => {
                AgoraError::SettlementFailed(format!(
                    "Insufficient on-chain balance: have {have}, need {need}"
                ))
            }
            other => AgoraError::SettlementFailed(format!("Solana transfer failed: {other}")),
        })?;

        Ok(SettlementReceipt {
            method: "solana-spl".to_string(),
            reference: Some(result.signature),
            on_chain_amount: amount,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Keypair;

    fn test_settlement() -> SolanaSettlement {
        let kp = Keypair::new();
        SolanaSettlement::new(
            "https://api.devnet.solana.com",
            kp.to_bytes().to_vec(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            6,
            1, // 1 credit = 1 base unit
        )
    }

    #[test]
    fn test_credits_to_base_units() {
        let s = test_settlement();
        assert_eq!(s.credits_to_base_units(100), 100);
        assert_eq!(s.credits_to_base_units(0), 0);
        assert_eq!(s.credits_to_base_units(-5), 0); // negative clamped to 0
    }

    #[test]
    fn test_credits_to_base_units_with_rate() {
        let mut s = test_settlement();
        s.base_units_per_credit = 1_000_000; // 1 credit = 1 USDC (6 decimals)
        assert_eq!(s.credits_to_base_units(1), 1_000_000);
        assert_eq!(s.credits_to_base_units(5), 5_000_000);
    }

    #[test]
    fn test_zero_amount_rejected() {
        let s = test_settlement();
        let result = s.settle("buyer", "seller", 0, "test-skill");
        assert!(result.is_err());
    }

    #[test]
    fn test_negative_credits_rejected() {
        let s = test_settlement();
        let result = s.settle("buyer", "seller", -10, "test-skill");
        assert!(result.is_err());
    }

    #[test]
    fn test_settle_rpc_failure() {
        // Settlement with a bogus RPC URL — should fail with RPC error wrapped in SettlementFailed
        let mut s = test_settlement();
        s.rpc_url = "http://127.0.0.1:1".to_string(); // nothing listening
        let result = s.settle("buyer", "11111111111111111111111111111111", 100, "test-skill");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Solana transfer failed"));
    }
}
