//! Settlement coordinator — unifies marketplace money-paths onto the canonical
//! `zeus-economy::TokenLedger` (SQLite, `~/.zeus/economy.db`).
//!
//! # Why this exists (#81a)
//!
//! Before this, three independent ledgers settled trades:
//!   1. `zeus-marketplace::TokenLedger` — in-memory (this crate)
//!   2. `zeus-api::marketplace_store`   — its own SQLite token ledger
//!   3. `zeus-agora`                    — its own sync engine
//!
//! None routed through the by-design single source of truth,
//! `zeus-economy::TokenLedger`. That is a money-path fork: balances can diverge
//! across surfaces. `#81a` unifies settlement so there is exactly one
//! settlement sink and exactly one durable ledger.
//!
//! # Conservative, off-by-default scaffold
//!
//! This is the **scaffold** cut: the coordinator is wired but **disabled by
//! default**. When disabled (the default), [`SettlementCoordinator::settle`] is
//! a no-op and the legacy in-memory path is left completely untouched — there
//! is **no live money-path change** until the operator explicitly flips the
//! flag. Enabling routes settlement atomically through
//! `zeus_economy::TokenLedger::settle_trade`.
//!
//! Flip via [`SettlementCoordinator::enabled`] (builder) or the
//! `ZEUS_UNIFY_MARKETPLACE` env var (`1`/`true`/`on` ⇒ enabled).

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};
use zeus_core::Result;
use zeus_economy::TokenLedger as EconomyLedger;

/// Default fee collector for marketplace settlements routed to the canonical
/// ledger. The treasury wallet receives protocol fees.
pub const DEFAULT_FEE_COLLECTOR: &str = "zeus-treasury";

/// Environment variable that flips the unified money-path on.
pub const UNIFY_ENV: &str = "ZEUS_UNIFY_MARKETPLACE";

/// Outcome of a settlement attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementOutcome {
    /// Coordinator is disabled (default). Legacy in-memory path remains
    /// authoritative; canonical ledger was not touched.
    Skipped,
    /// Settled atomically on the canonical economy ledger. Carries the
    /// post-settlement balances `(buyer, seller, fee_collector)`.
    Settled {
        buyer_balance: u64,
        seller_balance: u64,
        fee_balance: u64,
    },
}

impl SettlementOutcome {
    /// True when the canonical ledger actually settled the trade.
    pub fn is_settled(&self) -> bool {
        matches!(self, SettlementOutcome::Settled { .. })
    }
}

/// Coordinates marketplace settlement onto the canonical economy ledger.
///
/// Cheap to clone — the underlying economy ledger is path-backed and shared
/// behind an `Arc`. When `enabled` is false this is an inert pass-through.
#[derive(Clone)]
pub struct SettlementCoordinator {
    enabled: bool,
    fee_collector: String,
    ledger: Option<Arc<EconomyLedger>>,
}

impl SettlementCoordinator {
    /// Construct a **disabled** coordinator (the default, no-op) bound to the
    /// canonical economy database at `db_path`. Opening the ledger is lazy in
    /// effect: even when constructed, nothing settles until `enabled(true)`.
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let ledger = EconomyLedger::new(db_path)?;
        Ok(Self {
            enabled: false,
            fee_collector: DEFAULT_FEE_COLLECTOR.to_string(),
            ledger: Some(Arc::new(ledger)),
        })
    }

    /// Construct an inert coordinator with **no** backing ledger. Always skips.
    /// Useful for tests and for surfaces that never settle.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            fee_collector: DEFAULT_FEE_COLLECTOR.to_string(),
            ledger: None,
        }
    }

    /// Builder: set whether settlement is routed to the canonical ledger.
    /// Off by default — enabling is a deliberate, operator-gated action.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// Builder: override the fee-collector wallet.
    pub fn fee_collector(mut self, wallet: impl Into<String>) -> Self {
        self.fee_collector = wallet.into();
        self
    }

    /// Read the unify flag from the environment (`ZEUS_UNIFY_MARKETPLACE`).
    /// Recognizes `1`, `true`, `on`, `yes` (case-insensitive) as enabled.
    pub fn env_enabled() -> bool {
        std::env::var(UNIFY_ENV)
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "on" | "yes")
            })
            .unwrap_or(false)
    }

    /// Apply the environment flag onto this coordinator.
    pub fn with_env(mut self) -> Self {
        if Self::env_enabled() {
            self.enabled = true;
        }
        self
    }

    /// Whether this coordinator will route settlement to the canonical ledger.
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.ledger.is_some()
    }

    /// Settle a trade.
    ///
    /// When **disabled** (default): returns [`SettlementOutcome::Skipped`]
    /// without touching the canonical ledger — the legacy in-memory path stays
    /// authoritative and there is no money-path change.
    ///
    /// When **enabled**: routes atomically through
    /// `zeus_economy::TokenLedger::settle_trade`, the single settlement sink.
    pub fn settle(
        &self,
        buyer: &str,
        seller: &str,
        total_price: u64,
        fee_amount: u64,
        reference_id: impl Into<String>,
    ) -> Result<SettlementOutcome> {
        if !self.enabled {
            return Ok(SettlementOutcome::Skipped);
        }
        let Some(ledger) = self.ledger.as_ref() else {
            warn!(
                "SettlementCoordinator enabled but no canonical ledger bound; skipping settlement"
            );
            return Ok(SettlementOutcome::Skipped);
        };

        let (buyer_balance, seller_balance, fee_balance) = ledger.settle_trade(
            buyer,
            seller,
            &self.fee_collector,
            total_price,
            fee_amount,
            reference_id,
        )?;

        info!(
            buyer,
            seller,
            fee_collector = %self.fee_collector,
            total_price,
            fee_amount,
            "Settled trade on canonical economy ledger (unified money-path)"
        );

        Ok(SettlementOutcome::Settled {
            buyer_balance,
            seller_balance,
            fee_balance,
        })
    }
}

impl Default for SettlementCoordinator {
    /// Inert, disabled, no backing ledger. Safe everywhere.
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default_is_noop() {
        let coord = SettlementCoordinator::disabled();
        assert!(!coord.is_enabled());
        let out = coord
            .settle("buyer", "seller", 100, 5, "trade:1")
            .expect("disabled settle must not error");
        assert_eq!(out, SettlementOutcome::Skipped);
        assert!(!out.is_settled());
    }

    #[test]
    fn enabled_without_ledger_skips_safely() {
        let coord = SettlementCoordinator::disabled().enabled(true);
        // No backing ledger ⇒ still skips rather than panicking.
        assert!(!coord.is_enabled());
        let out = coord.settle("b", "s", 10, 0, "ref").unwrap();
        assert_eq!(out, SettlementOutcome::Skipped);
    }

    #[test]
    fn settles_on_canonical_ledger_when_enabled() {
        let dir = std::env::temp_dir().join(format!("zeus-settle-{}", uuid::Uuid::new_v4()));
        let db = dir.join("economy.db");
        let ledger = EconomyLedger::new(&db).expect("open economy ledger");
        // Fund the buyer on the canonical ledger.
        ledger
            .mint(
                "buyer",
                1_000,
                zeus_economy::TransactionReason::SystemGrant,
                "test-seed",
            )
            .expect("mint");

        let coord = SettlementCoordinator::new(&db)
            .expect("coordinator")
            .enabled(true)
            .fee_collector("zeus-treasury");
        assert!(coord.is_enabled());

        let out = coord
            .settle("buyer", "seller", 100, 10, "trade:42")
            .expect("settle");
        match out {
            SettlementOutcome::Settled {
                buyer_balance,
                seller_balance,
                fee_balance,
            } => {
                assert_eq!(buyer_balance, 900);
                assert_eq!(seller_balance, 90);
                assert_eq!(fee_balance, 10);
            }
            SettlementOutcome::Skipped => panic!("expected settlement, got skip"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_flag_parsing() {
        // SAFETY: single-threaded test; we set/unset our own env var and clean
        // up before returning. Edition-2024 marks env mutators unsafe.
        unsafe {
            // Not set ⇒ disabled.
            std::env::remove_var(UNIFY_ENV);
            assert!(!SettlementCoordinator::env_enabled());
            std::env::set_var(UNIFY_ENV, "1");
            assert!(SettlementCoordinator::env_enabled());
            std::env::set_var(UNIFY_ENV, "TRUE");
            assert!(SettlementCoordinator::env_enabled());
            std::env::set_var(UNIFY_ENV, "off");
            assert!(!SettlementCoordinator::env_enabled());
            std::env::remove_var(UNIFY_ENV);
        }
    }
}
