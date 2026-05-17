//! Settlement coordinator — two-phase saga for the Agora marketplace.
//!
//! Bridges three subsystems in a single atomic-feeling flow:
//!
//! | Phase | Action | Rollback on failure |
//! |-------|--------|---------------------|
//! | 1 | Debit buyer via `Marketplace::begin_transaction` | `abort_transaction` |
//! | 2 | Dispatch HTTP via `X402Client::post` | `abort_transaction` |
//! | 3a | Record trade in `TokenLedger::settle_trade` (SQLite) | non-fatal — log & continue |
//! | 3b | Credit seller via `Marketplace::commit_transaction` | — |
//!
//! Phase 3a failure is treated as non-fatal: the on-chain payment has already
//! been confirmed, so we log a warning for reconciliation and proceed to
//! Phase 3b rather than leaving the buyer debited with no settlement.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::marketplace::Marketplace;
use crate::{AgoraError, SkillListing, SkillTransaction};
use zeus_economy::TokenLedger;
use zeus_wallet::X402Client;

/// Two-phase settlement coordinator bridging the Agora marketplace,
/// the Zeus economy SQLite ledger, and the x402 payment protocol.
pub struct SettlementCoordinator {
    marketplace: Arc<Mutex<Marketplace>>,
    ledger: Arc<TokenLedger>,
    x402: Arc<X402Client>,
    fee_collector: String,
}

impl SettlementCoordinator {
    /// Create a new coordinator.
    pub fn new(
        marketplace: Arc<Mutex<Marketplace>>,
        ledger: Arc<TokenLedger>,
        x402: Arc<X402Client>,
        fee_collector: impl Into<String>,
    ) -> Self {
        Self {
            marketplace,
            ledger,
            x402,
            fee_collector: fee_collector.into(),
        }
    }

    /// Execute the full two-phase settlement saga for a skill purchase.
    ///
    /// Returns the finalized [`SkillTransaction`] on success.
    pub async fn settle(
        &self,
        buyer_id: &str,
        listing: &SkillListing,
        endpoint: &str,
        payload: &serde_json::Value,
    ) -> Result<SkillTransaction, AgoraError> {
        // ── Phase 1: Reserve buyer credits ────────────────────────────────
        let tx_id = {
            let mut mp = self.marketplace.lock().await;
            mp.begin_transaction(buyer_id, listing)?
        };

        tracing::debug!(
            tx_id = %tx_id,
            buyer = %buyer_id,
            seller = %listing.agent_id,
            credits = listing.price_credits,
            "Phase 1 complete — buyer debited"
        );

        // ── Phase 2: Dispatch x402 HTTP payment ───────────────────────────
        let body = payload.to_string();
        let reference = match self.x402.post(endpoint, &body).await {
            Ok((_, Some(receipt))) if receipt.success => {
                tracing::info!(
                    tx_id = %tx_id,
                    signature = %receipt.signature,
                    "Phase 2 complete — x402 payment confirmed"
                );
                Some(receipt.signature)
            }
            Ok((_, Some(receipt))) => {
                // Payment was attempted but the server reported failure.
                let mut mp = self.marketplace.lock().await;
                let _ = mp.abort_transaction(tx_id);
                return Err(AgoraError::SettlementFailed(format!(
                    "x402 payment not accepted (retry_status {})",
                    receipt.retry_status
                )));
            }
            Ok((_, None)) => {
                // Server accepted the request without issuing a 402 challenge.
                tracing::debug!(
                    tx_id = %tx_id,
                    "Phase 2 — x402 not required, proceeding without on-chain payment"
                );
                None
            }
            Err(e) => {
                let mut mp = self.marketplace.lock().await;
                let _ = mp.abort_transaction(tx_id);
                return Err(AgoraError::SettlementFailed(e.to_string()));
            }
        };

        // ── Phase 3a: Record in SQLite ledger (non-fatal) ─────────────────
        let price = listing.price_credits as u64;
        let commission = ((price as f64 * 0.05).ceil()) as u64;
        let fee_collector = self.fee_collector.clone();
        let seller_id = listing.agent_id.clone();
        let buyer_id_owned = buyer_id.to_string();
        let ref_str = reference.clone().unwrap_or_default();
        let ledger = Arc::clone(&self.ledger);

        let ledger_result = tokio::task::spawn_blocking(move || {
            ledger.settle_trade(
                &buyer_id_owned,
                &seller_id,
                &fee_collector,
                price,
                commission,
                ref_str,
            )
        })
        .await
        .map_err(|e| format!("spawn_blocking join error: {e}"))
        .and_then(|result| result.map_err(|e| e.to_string()));

        if let Err(ref e) = ledger_result {
            tracing::warn!(
                tx_id = %tx_id,
                error = %e,
                "Phase 3a — ledger settle_trade failed; on-chain payment confirmed, proceeding to commit"
            );
        }

        // ── Phase 3b: Commit — credit seller and finalize transaction ─────
        let tx = {
            let mut mp = self.marketplace.lock().await;
            mp.commit_transaction(tx_id, reference)?
        };

        tracing::info!(
            tx_id = %tx_id,
            buyer = %buyer_id,
            seller = %listing.agent_id,
            credits = listing.price_credits,
            "settlement complete"
        );

        Ok(tx)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::marketplace::InMemorySettlement;
    use crate::{
        Marketplace, MarketplaceConfig, SkillListing, SkillTransaction, TransactionStatus,
    };

    fn test_listing(agent: &str, skill: &str, price: i64) -> SkillListing {
        SkillListing::new(
            agent,
            skill,
            format!("{skill} description"),
            price,
            "{}",
            "{}",
        )
    }

    /// Smoke-test the saga-phase methods directly (no X402 / ledger needed).
    #[test]
    fn test_begin_commit_roundtrip() {
        let mut mp = Marketplace::with_settlement(
            MarketplaceConfig::default(),
            Box::new(InMemorySettlement),
        );

        let listing = test_listing("seller-1", "translate", 100);
        mp.register_wallet("buyer-1", 500);
        mp.list_skill(listing.clone()).unwrap();

        // Phase 1
        let tx_id = mp.begin_transaction("buyer-1", &listing).unwrap();
        assert_eq!(mp.balance("buyer-1"), Some(400)); // 500 - 100

        // Phase 3b
        let tx = mp
            .commit_transaction(tx_id, Some("sig-abc".to_string()))
            .unwrap();
        assert_eq!(tx.status, TransactionStatus::Completed);
        assert_eq!(tx.settlement_reference.as_deref(), Some("sig-abc"));

        // Commission = ceil(100 * 0.05) = 5 → seller gets 95
        assert_eq!(mp.balance("seller-1"), Some(95));
        assert_eq!(mp.transaction_count(), 1);
    }

    #[test]
    fn test_begin_abort_refunds_buyer() {
        let mut mp = Marketplace::with_defaults();
        let listing = test_listing("seller-2", "analyze", 50);
        mp.register_wallet("buyer-2", 200);

        let tx_id = mp.begin_transaction("buyer-2", &listing).unwrap();
        assert_eq!(mp.balance("buyer-2"), Some(150));

        mp.abort_transaction(tx_id).unwrap();
        assert_eq!(mp.balance("buyer-2"), Some(200)); // refunded
    }

    #[test]
    fn test_begin_insufficient_funds() {
        let mut mp = Marketplace::with_defaults();
        let listing = test_listing("seller-3", "code", 1000);
        mp.register_wallet("buyer-3", 10);

        let err = mp.begin_transaction("buyer-3", &listing).unwrap_err();
        assert!(matches!(err, crate::AgoraError::InsufficientCredits { .. }));
        assert_eq!(mp.balance("buyer-3"), Some(10)); // unchanged
    }

    #[test]
    fn test_commit_unknown_tx_id() {
        let mut mp = Marketplace::with_defaults();
        let err = mp
            .commit_transaction(uuid::Uuid::new_v4(), None)
            .unwrap_err();
        assert!(matches!(err, crate::AgoraError::TransactionNotFound(_)));
    }

    /// Full async integration: SettlementCoordinator with an always-accepting
    /// mock (no 402 issued) verifies the complete saga path.
    #[tokio::test]
    async fn test_coordinator_no_x402_path() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let mp = Arc::new(Mutex::new(Marketplace::with_settlement(
            MarketplaceConfig::default(),
            Box::new(InMemorySettlement),
        )));

        {
            let mut guard = mp.lock().await;
            guard.register_wallet("buyer-async", 1000);
            let listing = test_listing("seller-async", "summarize", 100);
            guard.list_skill(listing).unwrap();
        }

        // Use a temp dir for both the wallet and ledger so tests don't share state.
        let tmp = tempfile::tempdir().unwrap();

        // Build a real X402Client pointing at the mock server.
        let keypair = zeus_wallet::WalletKeypair::generate(
            tmp.path().join("wallet"),
            "test-key",
            "solana-devnet",
        )
        .unwrap();
        let x402_cfg = zeus_wallet::X402Config {
            max_amount: 1_000_000,
            allowed_networks: vec!["solana-devnet".to_string()],
            allowed_tokens: vec!["ZEUS".to_string()],
        };
        let x402 = Arc::new(zeus_wallet::X402Client::new(keypair, x402_cfg));

        let ledger = Arc::new(zeus_economy::TokenLedger::new(tmp.path().join("test.db")).unwrap());

        let coordinator =
            super::SettlementCoordinator::new(mp.clone(), ledger, x402, "fee-collector");

        let listing = test_listing("seller-async", "summarize", 100);
        let endpoint = format!("{}/skill/summarize", server.uri());
        let tx: SkillTransaction = coordinator
            .settle("buyer-async", &listing, &endpoint, &serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(tx.status, TransactionStatus::Completed);
        assert!(tx.settlement_reference.is_none()); // no 402 was issued

        let guard = mp.lock().await;
        assert_eq!(guard.balance("buyer-async"), Some(900)); // 1000 - 100
        assert_eq!(guard.balance("seller-async"), Some(95)); // 100 - 5% commission
    }
}
