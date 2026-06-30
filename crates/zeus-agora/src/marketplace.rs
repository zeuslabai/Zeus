//! Marketplace Engine for the Agora.
//!
//! Provides skill discovery, reputation tracking, and dispute resolution
//! for the agent skill marketplace:
//!
//! - **Marketplace** — central registry for skill listings with search/filter
//! - **ReputationTracker** — per-agent reputation scores based on transaction history
//! - **DisputeManager** — dispute filing, evidence collection, and resolution
//! - **CategoryIndex** — category-based skill organization with tag matching

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;
use zeus_wallet::x402::X402Config;

use crate::{AgentWallet, AgoraError, SkillListing, SkillTransaction, TransactionStatus};

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for the marketplace engine.
#[derive(Debug, Clone)]
pub struct MarketplaceConfig {
    /// Minimum reputation score to list skills (0.0–1.0).
    pub min_listing_reputation: f64,
    /// Number of recent transactions used for reputation calculation.
    pub reputation_window: usize,
    /// Maximum number of open disputes per agent.
    pub max_open_disputes: usize,
    /// Auto-resolve disputes older than this many seconds.
    pub dispute_timeout_secs: u64,
    /// Commission rate taken from each transaction (0.0–1.0).
    pub commission_rate: f64,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            min_listing_reputation: 0.0,
            reputation_window: 100,
            max_open_disputes: 5,
            dispute_timeout_secs: 86400, // 24 hours
            commission_rate: 0.05,       // 5%
        }
    }
}

// ============================================================================
// Reputation
// ============================================================================

/// Outcome of a transaction for reputation scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionOutcome {
    Success,
    Failure,
    Disputed,
    Refunded,
}

impl From<TransactionStatus> for TransactionOutcome {
    fn from(status: TransactionStatus) -> Self {
        match status {
            TransactionStatus::Completed => TransactionOutcome::Success,
            TransactionStatus::Failed => TransactionOutcome::Failure,
            TransactionStatus::Refunded => TransactionOutcome::Refunded,
            TransactionStatus::Pending | TransactionStatus::InProgress => {
                TransactionOutcome::Success
            }
        }
    }
}

/// A single reputation data point.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ReputationEntry {
    outcome: TransactionOutcome,
    credits: i64,
    recorded_at: DateTime<Utc>,
}

/// Aggregated reputation score for an agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReputationScore {
    /// Agent identifier.
    pub agent_id: String,
    /// Overall reputation score (0.0–1.0).
    pub score: f64,
    /// Total transactions evaluated.
    pub total_transactions: usize,
    /// Number of successful transactions.
    pub successes: usize,
    /// Number of failures.
    pub failures: usize,
    /// Number of disputes.
    pub disputes: usize,
    /// Total credits transacted.
    pub total_credits: i64,
}

/// Tracks per-agent reputation based on transaction outcomes.
pub struct ReputationTracker {
    entries: HashMap<String, Vec<ReputationEntry>>,
    window: usize,
}

impl ReputationTracker {
    /// Create a new tracker with the given rolling window size.
    pub fn new(window: usize) -> Self {
        Self {
            entries: HashMap::new(),
            window,
        }
    }

    /// Record a transaction outcome for an agent.
    pub fn record(&mut self, agent_id: &str, outcome: TransactionOutcome, credits: i64) {
        let entries = self.entries.entry(agent_id.to_string()).or_default();
        entries.push(ReputationEntry {
            outcome,
            credits,
            recorded_at: Utc::now(),
        });
        // Keep only the most recent `window` entries
        if entries.len() > self.window {
            let excess = entries.len() - self.window;
            entries.drain(..excess);
        }
    }

    /// All agent IDs with at least one recorded reputation entry.
    pub fn agent_ids(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Calculate the reputation score for an agent.
    pub fn score(&self, agent_id: &str) -> ReputationScore {
        let empty = vec![];
        let entries = self.entries.get(agent_id).unwrap_or(&empty);

        let total = entries.len();
        let mut successes = 0usize;
        let mut failures = 0usize;
        let mut disputes = 0usize;
        let mut total_credits = 0i64;

        for entry in entries {
            total_credits += entry.credits;
            match entry.outcome {
                TransactionOutcome::Success => successes += 1,
                TransactionOutcome::Failure => failures += 1,
                TransactionOutcome::Disputed => disputes += 1,
                TransactionOutcome::Refunded => failures += 1,
            }
        }

        let score = if total == 0 {
            0.5 // neutral default
        } else {
            // Weighted: success=1.0, dispute=0.3, failure/refund=0.0
            let weighted_sum = successes as f64 + disputes as f64 * 0.3;
            weighted_sum / total as f64
        };

        ReputationScore {
            agent_id: agent_id.to_string(),
            score,
            total_transactions: total,
            successes,
            failures,
            disputes,
            total_credits,
        }
    }

    /// List all tracked agent IDs.
    pub fn tracked_agents(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Clear all reputation data for an agent.
    pub fn clear(&mut self, agent_id: &str) {
        self.entries.remove(agent_id);
    }
}

// ============================================================================
// Disputes
// ============================================================================

/// Status of a dispute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisputeStatus {
    Open,
    UnderReview,
    ResolvedBuyer,
    ResolvedSeller,
    Dismissed,
    TimedOut,
}

/// A dispute between a buyer and seller.
#[derive(Debug, Clone)]
pub struct Dispute {
    pub id: String,
    pub transaction_id: Uuid,
    pub filed_by: String,
    pub against: String,
    pub reason: String,
    pub evidence: Vec<String>,
    pub status: DisputeStatus,
    pub resolution_note: Option<String>,
    pub filed_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Manages disputes between agents.
pub struct DisputeManager {
    disputes: Vec<Dispute>,
    max_open_per_agent: usize,
    timeout_secs: u64,
}

impl DisputeManager {
    /// Create a new dispute manager.
    pub fn new(max_open_per_agent: usize, timeout_secs: u64) -> Self {
        Self {
            disputes: Vec::new(),
            max_open_per_agent,
            timeout_secs,
        }
    }

    /// File a new dispute.
    pub fn file(
        &mut self,
        transaction_id: Uuid,
        filed_by: &str,
        against: &str,
        reason: &str,
    ) -> Result<String, AgoraError> {
        // Check open dispute limit
        let open_count = self
            .disputes
            .iter()
            .filter(|d| d.filed_by == filed_by && d.status == DisputeStatus::Open)
            .count();
        if open_count >= self.max_open_per_agent {
            return Err(AgoraError::InvalidAmount(self.max_open_per_agent as i64));
        }

        let id = Uuid::new_v4().to_string();
        self.disputes.push(Dispute {
            id: id.clone(),
            transaction_id,
            filed_by: filed_by.to_string(),
            against: against.to_string(),
            reason: reason.to_string(),
            evidence: Vec::new(),
            status: DisputeStatus::Open,
            resolution_note: None,
            filed_at: Utc::now(),
            resolved_at: None,
        });
        Ok(id)
    }

    /// Add evidence to a dispute.
    pub fn add_evidence(&mut self, dispute_id: &str, evidence: &str) -> Result<(), AgoraError> {
        let dispute = self
            .disputes
            .iter_mut()
            .find(|d| d.id == dispute_id)
            .ok_or_else(|| AgoraError::TransactionNotFound(Uuid::nil()))?;

        if dispute.status != DisputeStatus::Open && dispute.status != DisputeStatus::UnderReview {
            return Err(AgoraError::InvalidAmount(0)); // dispute already resolved
        }

        dispute.evidence.push(evidence.to_string());
        dispute.status = DisputeStatus::UnderReview;
        Ok(())
    }

    /// Resolve a dispute in favor of a party.
    pub fn resolve(
        &mut self,
        dispute_id: &str,
        in_favor_of_buyer: bool,
        note: &str,
    ) -> Result<(), AgoraError> {
        let dispute = self
            .disputes
            .iter_mut()
            .find(|d| d.id == dispute_id)
            .ok_or_else(|| AgoraError::TransactionNotFound(Uuid::nil()))?;

        dispute.status = if in_favor_of_buyer {
            DisputeStatus::ResolvedBuyer
        } else {
            DisputeStatus::ResolvedSeller
        };
        dispute.resolution_note = Some(note.to_string());
        dispute.resolved_at = Some(Utc::now());
        Ok(())
    }

    /// Dismiss a dispute.
    pub fn dismiss(&mut self, dispute_id: &str, note: &str) -> Result<(), AgoraError> {
        let dispute = self
            .disputes
            .iter_mut()
            .find(|d| d.id == dispute_id)
            .ok_or_else(|| AgoraError::TransactionNotFound(Uuid::nil()))?;

        dispute.status = DisputeStatus::Dismissed;
        dispute.resolution_note = Some(note.to_string());
        dispute.resolved_at = Some(Utc::now());
        Ok(())
    }

    /// Auto-resolve timed-out disputes.
    pub fn timeout_expired(&mut self) -> Vec<String> {
        let now = Utc::now();
        let timeout = chrono::Duration::seconds(self.timeout_secs as i64);
        let mut timed_out = Vec::new();

        for dispute in &mut self.disputes {
            if (dispute.status == DisputeStatus::Open
                || dispute.status == DisputeStatus::UnderReview)
                && now.signed_duration_since(dispute.filed_at) > timeout
            {
                dispute.status = DisputeStatus::TimedOut;
                dispute.resolution_note = Some("Auto-timed out".to_string());
                dispute.resolved_at = Some(now);
                timed_out.push(dispute.id.clone());
            }
        }
        timed_out
    }

    /// Get a dispute by ID.
    pub fn get(&self, dispute_id: &str) -> Option<&Dispute> {
        self.disputes.iter().find(|d| d.id == dispute_id)
    }

    /// List disputes for an agent (as filer or target).
    pub fn list_for_agent(&self, agent_id: &str) -> Vec<&Dispute> {
        self.disputes
            .iter()
            .filter(|d| d.filed_by == agent_id || d.against == agent_id)
            .collect()
    }

    /// Count open disputes.
    pub fn open_count(&self) -> usize {
        self.disputes
            .iter()
            .filter(|d| d.status == DisputeStatus::Open || d.status == DisputeStatus::UnderReview)
            .count()
    }

    /// Total disputes.
    pub fn total(&self) -> usize {
        self.disputes.len()
    }
}

// ============================================================================
// Category Index
// ============================================================================

/// A marketplace category for organizing skills.
#[derive(Debug, Clone)]
pub struct Category {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// Index for categorizing and discovering skills by tags.
pub struct CategoryIndex {
    categories: Vec<Category>,
}

impl CategoryIndex {
    /// Create a new empty category index.
    pub fn new() -> Self {
        Self {
            categories: Vec::new(),
        }
    }

    /// Create a category index with default categories.
    pub fn with_defaults() -> Self {
        let mut idx = Self::new();
        idx.add(Category {
            name: "Code".to_string(),
            description: "Code generation, review, and analysis".to_string(),
            tags: vec![
                "code".into(),
                "programming".into(),
                "review".into(),
                "analysis".into(),
            ],
        });
        idx.add(Category {
            name: "Data".to_string(),
            description: "Data processing, transformation, and analytics".to_string(),
            tags: vec![
                "data".into(),
                "analytics".into(),
                "transform".into(),
                "etl".into(),
            ],
        });
        idx.add(Category {
            name: "Language".to_string(),
            description: "Translation, summarization, and text processing".to_string(),
            tags: vec![
                "language".into(),
                "translation".into(),
                "summarization".into(),
                "text".into(),
            ],
        });
        idx.add(Category {
            name: "Security".to_string(),
            description: "Security auditing, scanning, and compliance".to_string(),
            tags: vec![
                "security".into(),
                "audit".into(),
                "compliance".into(),
                "scanning".into(),
            ],
        });
        idx
    }

    /// Add a category.
    pub fn add(&mut self, category: Category) {
        self.categories.push(category);
    }

    /// Find categories matching any of the given tags.
    pub fn match_tags(&self, tags: &[String]) -> Vec<&Category> {
        self.categories
            .iter()
            .filter(|cat| cat.tags.iter().any(|t| tags.contains(t)))
            .collect()
    }

    /// Get a category by name.
    pub fn get(&self, name: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.name == name)
    }

    /// List all categories.
    pub fn list(&self) -> &[Category] {
        &self.categories
    }

    /// Count categories.
    pub fn count(&self) -> usize {
        self.categories.len()
    }
}

impl Default for CategoryIndex {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Settlement
// ============================================================================

/// Result of a settlement operation.
#[derive(Debug, Clone)]
pub struct SettlementReceipt {
    /// Settlement method: `"in-memory"` or `"x402"`.
    pub method: String,
    /// On-chain payment reference or signature (`None` for in-memory).
    pub reference: Option<String>,
    /// Amount settled in micro-USDC (0 for in-memory).
    pub on_chain_amount: u64,
}

/// Pluggable settlement backend for the marketplace.
///
/// Implement this trait to swap between in-memory credit accounting and
/// on-chain payment networks (e.g. x402 / USDC on Solana via zeus-wallet).
///
/// Called inside `execute_transaction()` after buyer funds are validated but
/// before credits are transferred to the seller.  On `Err` the buyer is
/// refunded and the transaction is logged as `Failed`.
pub trait SettlementProvider: Send + Sync {
    fn settle(
        &self,
        buyer_id: &str,
        seller_id: &str,
        amount_credits: i64,
        skill_name: &str,
    ) -> Result<SettlementReceipt, AgoraError>;
}

/// In-memory settlement — credit accounting only, no external calls.
///
/// This is the default backend; credits are managed entirely within the
/// `Marketplace` wallet map.
pub struct InMemorySettlement;

impl SettlementProvider for InMemorySettlement {
    fn settle(
        &self,
        _buyer_id: &str,
        _seller_id: &str,
        _amount_credits: i64,
        _skill_name: &str,
    ) -> Result<SettlementReceipt, AgoraError> {
        Ok(SettlementReceipt {
            method: "in-memory".to_string(),
            reference: None,
            on_chain_amount: 0,
        })
    }
}

/// Canonical settlement — mirrors each agora trade onto the by-design single
/// source of truth, `zeus_economy::TokenLedger` (SQLite, `~/.zeus/economy.db`).
///
/// # Why this exists (#81a Phase 3)
///
/// Agora's `execute_transaction` settles credits in the in-memory wallet map —
/// a *second money path* that can diverge from the canonical economy ledger.
/// This provider closes that fork by **mirroring** every completed trade onto
/// `economy.db`, making agora a read-projection of the one true ledger rather
/// than an independent settlement sink.
///
/// # Conservative, off-by-default, mirror-not-gate
///
/// - **Disabled by default.** Construct via [`CanonicalSettlement::from_env`];
///   unless `ZEUS_UNIFY_MARKETPLACE` is set (`1`/`true`/`on`/`yes`), this is an
///   inert no-op and the in-memory path remains fully authoritative.
/// - **Mirror, not gate.** Even when enabled, a canonical-ledger error is
///   logged loud but **never** returned as `Err`. Agora's in-memory wallet
///   accounting stays authoritative for trade success/failure, so a transient
///   `economy.db` hiccup can never reverse a committed agora trade or strand a
///   buyer's debit. (Mirrors the Phase-2 `marketplace_store` safety contract.)
/// - **Fail-safe construction.** If `economy.db` can't be opened, falls back to
///   a disabled no-op; the legacy path can't break.
pub struct CanonicalSettlement {
    enabled: bool,
    fee_collector: String,
    ledger: Option<std::sync::Arc<zeus_economy::TokenLedger>>,
    commission_rate: f64,
}

impl CanonicalSettlement {
    /// Default fee-collector wallet (treasury) for unified settlement.
    const DEFAULT_FEE_COLLECTOR: &'static str = "zeus-treasury";

    /// Read the unify flag from the environment (`ZEUS_UNIFY_MARKETPLACE`).
    /// Recognizes `1`, `true`, `on`, `yes` (case-insensitive) as enabled.
    pub fn env_enabled() -> bool {
        std::env::var("ZEUS_UNIFY_MARKETPLACE")
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "on" | "yes")
            })
            .unwrap_or(false)
    }

    /// An inert provider that always skips. Used as the fail-safe fallback and
    /// for surfaces that never settle canonically.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            fee_collector: Self::DEFAULT_FEE_COLLECTOR.to_string(),
            ledger: None,
            commission_rate: 0.0,
        }
    }

    /// Build a canonical-settlement provider backed by the economy ledger at
    /// `db_path`, honoring the `ZEUS_UNIFY_MARKETPLACE` flag.
    ///
    /// Off by default. If the flag is unset, or the ledger can't be opened,
    /// returns an inert no-op so the legacy in-memory path is never at risk.
    pub fn from_env(db_path: impl Into<std::path::PathBuf>, commission_rate: f64) -> Self {
        if !Self::env_enabled() {
            return Self::disabled();
        }
        match zeus_economy::TokenLedger::new(db_path.into()) {
            Ok(ledger) => Self {
                enabled: true,
                fee_collector: Self::DEFAULT_FEE_COLLECTOR.to_string(),
                ledger: Some(std::sync::Arc::new(ledger)),
                commission_rate,
            },
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "CanonicalSettlement: economy ledger open failed; \
                     falling back to disabled (legacy in-memory path authoritative)"
                );
                Self::disabled()
            }
        }
    }

    /// Whether canonical mirroring is live (enabled *and* ledger bound).
    pub fn is_enabled(&self) -> bool {
        self.enabled && self.ledger.is_some()
    }

    /// Override the fee-collector (treasury) wallet.
    pub fn fee_collector(mut self, wallet: impl Into<String>) -> Self {
        self.fee_collector = wallet.into();
        self
    }
}

impl SettlementProvider for CanonicalSettlement {
    fn settle(
        &self,
        buyer_id: &str,
        seller_id: &str,
        amount_credits: i64,
        skill_name: &str,
    ) -> Result<SettlementReceipt, AgoraError> {
        // Disabled (default) ⇒ pure no-op; in-memory path stays authoritative.
        let Some(ledger) = self.ledger.as_ref().filter(|_| self.enabled) else {
            return Ok(SettlementReceipt {
                method: "in-memory".to_string(),
                reference: None,
                on_chain_amount: 0,
            });
        };

        let total = amount_credits.max(0) as u64;
        let fee = (amount_credits.max(0) as f64 * self.commission_rate).ceil() as u64;
        let reference_id = format!("agora:{}:{}:{}", buyer_id, seller_id, skill_name);

        // Mirror, not gate: on canonical error, log loud and return Ok so the
        // in-memory wallet path remains authoritative and the trade survives.
        match ledger.settle_trade(buyer_id, seller_id, &self.fee_collector, total, fee, reference_id)
        {
            Ok((buyer_bal, seller_bal, fee_bal)) => {
                tracing::info!(
                    buyer = buyer_id,
                    seller = seller_id,
                    skill = skill_name,
                    total,
                    fee,
                    buyer_bal,
                    seller_bal,
                    fee_bal,
                    "Agora trade mirrored onto canonical economy ledger (#81a unified money-path)"
                );
                Ok(SettlementReceipt {
                    method: "canonical-economy".to_string(),
                    reference: Some(format!("economy:{seller_id}:{buyer_bal}")),
                    on_chain_amount: 0,
                })
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    buyer = buyer_id,
                    seller = seller_id,
                    skill = skill_name,
                    "Canonical mirror failed; in-memory path remains authoritative \
                     (trade NOT reversed). Logged for reconciliation."
                );
                // Non-fatal: in-memory accounting already succeeded upstream.
                Ok(SettlementReceipt {
                    method: "in-memory-fallback".to_string(),
                    reference: None,
                    on_chain_amount: 0,
                })
            }
        }
    }
}

/// x402 settlement — issues an x402 USDC payment on each skill purchase.
///
/// Validates the converted micro-USDC amount against [`X402Config::max_amount`]
/// and returns a receipt carrying the payment intent.  Actual HTTP dispatch
/// via `zeus_wallet::x402::X402Client` must be performed by the async call
/// site; call [`X402Settlement::x402_config`] to obtain the config needed to
/// build the client.
///
/// Conversion: `credits × 1_000_000 / credits_per_usdc = micro-USDC`.
pub struct X402Settlement {
    /// x402 payment policy (amount cap, allowed networks and tokens).
    pub config: X402Config,
    /// Seller's resource endpoint (base URL that the buyer's X402Client hits).
    pub seller_endpoint: String,
    /// How many credits equal 1 USDC (conversion rate).
    pub credits_per_usdc: u64,
}

impl X402Settlement {
    /// Create a new x402 settlement backend.
    pub fn new(
        config: X402Config,
        seller_endpoint: impl Into<String>,
        credits_per_usdc: u64,
    ) -> Self {
        Self {
            config,
            seller_endpoint: seller_endpoint.into(),
            credits_per_usdc,
        }
    }

    /// Convert a credit amount to micro-USDC (1 USDC = 1 000 000 µUSDC).
    pub fn to_micro_usdc(&self, credits: i64) -> u64 {
        if self.credits_per_usdc == 0 {
            return 0;
        }
        (credits.max(0) as u64)
            .saturating_mul(1_000_000)
            .saturating_div(self.credits_per_usdc)
    }

    /// Return the x402 config.  Use this to build an `X402Client` in async
    /// contexts for actual on-chain payment dispatch.
    pub fn x402_config(&self) -> &X402Config {
        &self.config
    }
}

impl SettlementProvider for X402Settlement {
    fn settle(
        &self,
        buyer_id: &str,
        _seller_id: &str,
        amount_credits: i64,
        skill_name: &str,
    ) -> Result<SettlementReceipt, AgoraError> {
        let micro_usdc = self.to_micro_usdc(amount_credits);

        if micro_usdc > self.config.max_amount {
            return Err(AgoraError::SettlementFailed(format!(
                "x402: {}µUSDC exceeds policy cap of {}µUSDC",
                micro_usdc, self.config.max_amount,
            )));
        }

        if self.config.allowed_networks.iter().all(|n| n.is_empty()) {
            return Err(AgoraError::SettlementFailed(
                "x402: no allowed networks configured".to_string(),
            ));
        }

        // Return the payment intent reference.  The async call site is
        // responsible for dispatching the actual HTTP request via X402Client.
        let reference = format!(
            "x402://{}?buyer={}&skill={}&amount={}µUSDC&network={}",
            self.seller_endpoint,
            buyer_id,
            skill_name,
            micro_usdc,
            self.config
                .allowed_networks
                .first()
                .map(|s| s.as_str())
                .unwrap_or("unknown"),
        );

        Ok(SettlementReceipt {
            method: "x402".to_string(),
            reference: Some(reference),
            on_chain_amount: micro_usdc,
        })
    }
}

// ============================================================================
// Transaction Log
// ============================================================================

/// Query filters for transaction log searches.
#[derive(Debug, Clone, Default)]
pub struct TransactionFilter {
    /// Match transactions where the agent is buyer or seller.
    pub agent_id: Option<String>,
    /// Match by exact skill name.
    pub skill_name: Option<String>,
    /// Include only transactions created at or after this time.
    pub after: Option<DateTime<Utc>>,
    /// Include only transactions created strictly before this time.
    pub before: Option<DateTime<Utc>>,
    /// Match by transaction status.
    pub status: Option<TransactionStatus>,
    /// Cap results (most recent first).
    pub limit: Option<usize>,
}

/// Append-only log of all finalized transactions (Completed or Failed).
///
/// Entries are stored in insertion order; queries return most-recent-first.
pub struct TransactionLog {
    entries: Vec<SkillTransaction>,
}

impl TransactionLog {
    /// Create an empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Append a finalized transaction. Only call after status is Completed or Failed.
    pub fn append(&mut self, tx: SkillTransaction) {
        self.entries.push(tx);
    }

    /// Query transactions matching the given filter. Returns most recent first.
    pub fn query(&self, filter: &TransactionFilter) -> Vec<&SkillTransaction> {
        let mut results: Vec<&SkillTransaction> = self
            .entries
            .iter()
            .filter(|tx| {
                if let Some(ref aid) = filter.agent_id
                    && tx.buyer_agent_id != *aid
                    && tx.seller_agent_id != *aid
                {
                    return false;
                }
                if let Some(ref skill) = filter.skill_name
                    && tx.skill_name != *skill
                {
                    return false;
                }
                if let Some(after) = filter.after
                    && tx.created_at < after
                {
                    return false;
                }
                if let Some(before) = filter.before
                    && tx.created_at >= before
                {
                    return false;
                }
                if let Some(status) = filter.status
                    && tx.status != status
                {
                    return false;
                }
                true
            })
            .collect();

        // Most recent first
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }

        results
    }

    /// Total number of logged entries.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Sum of credits transferred across all logged entries.
    pub fn total_volume(&self) -> i64 {
        self.entries.iter().map(|t| t.credits_transferred).sum()
    }
}

impl Default for TransactionLog {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Marketplace Engine
// ============================================================================

/// Search criteria for marketplace queries.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Text to search in skill name and description.
    pub text: Option<String>,
    /// Filter by agent ID.
    pub agent_id: Option<String>,
    /// Filter by tags (any match).
    pub tags: Vec<String>,
    /// Filter by capabilities (any match).
    pub capabilities: Vec<String>,
    /// Maximum price in credits.
    pub max_price: Option<i64>,
    /// Minimum success rate (0.0–1.0).
    pub min_success_rate: Option<f64>,
    /// Maximum results.
    pub limit: Option<usize>,
}

/// A search result with relevance scoring.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub listing: SkillListing,
    pub relevance: f64,
    pub reputation: f64,
}

/// The main marketplace engine coordinating listings, reputation, and disputes.
pub struct Marketplace {
    config: MarketplaceConfig,
    listings: HashMap<String, SkillListing>, // key: "{agent_id}/{skill_name}"
    wallets: HashMap<String, AgentWallet>,
    reputation: ReputationTracker,
    disputes: DisputeManager,
    categories: CategoryIndex,
    transaction_log: TransactionLog,
    settlement: Box<dyn SettlementProvider>,
    /// In-flight reservations awaiting Phase 3b commit or abort.
    pending: HashMap<Uuid, SkillTransaction>,
}

impl Marketplace {
    /// Create a new marketplace with the given configuration.
    ///
    /// Uses [`InMemorySettlement`] by default.  For on-chain x402 settlement
    /// use [`Marketplace::with_settlement`] instead.
    pub fn new(config: MarketplaceConfig) -> Self {
        Self::with_settlement(config, Box::new(InMemorySettlement))
    }

    /// Create a marketplace with a custom settlement provider.
    ///
    /// ```ignore
    /// let mp = Marketplace::with_settlement(
    ///     MarketplaceConfig::default(),
    ///     Box::new(X402Settlement::new(X402Config::default(), "https://seller.example", 100)),
    /// );
    /// ```
    pub fn with_settlement(
        config: MarketplaceConfig,
        settlement: Box<dyn SettlementProvider>,
    ) -> Self {
        Self {
            reputation: ReputationTracker::new(config.reputation_window),
            disputes: DisputeManager::new(config.max_open_disputes, config.dispute_timeout_secs),
            config,
            listings: HashMap::new(),
            wallets: HashMap::new(),
            categories: CategoryIndex::with_defaults(),
            transaction_log: TransactionLog::new(),
            settlement,
            pending: HashMap::new(),
        }
    }

    /// Create a marketplace with default configuration and in-memory settlement.
    pub fn with_defaults() -> Self {
        Self::new(MarketplaceConfig::default())
    }

    // -- Wallet management --------------------------------------------------

    /// Register an agent wallet with an initial balance.
    pub fn register_wallet(&mut self, agent_id: &str, initial_balance: i64) {
        self.wallets
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentWallet::new(agent_id, initial_balance));
    }

    /// Get an agent's wallet balance.
    pub fn balance(&self, agent_id: &str) -> Option<i64> {
        self.wallets.get(agent_id).map(|w| w.balance)
    }

    /// Set (insert-or-overwrite) an agent's wallet balance.
    ///
    /// Unlike [`register_wallet`](Self::register_wallet), which only seeds a
    /// wallet when absent, this overwrites the balance unconditionally. Intended
    /// for idempotent boot-time hydration from a persistence store, where the
    /// authoritative balance lives in SQLite and the in-memory wallet must
    /// reflect it after a restart.
    pub fn set_balance(&mut self, agent_id: &str, balance: i64) {
        self.wallets
            .entry(agent_id.to_string())
            .and_modify(|w| w.balance = balance)
            .or_insert_with(|| AgentWallet::new(agent_id, balance));
    }

    /// Seed an agent's reputation from a known history of successful and failed
    /// trades.
    ///
    /// Replays `successes` success outcomes and `failures` failure outcomes
    /// through the reputation tracker. Intended for boot-time hydration from a
    /// persistence store that records aggregate trade counts rather than the
    /// individual transactions. `credits` defaults to 0 per replayed outcome
    /// since the hydration source tracks counts, not per-trade amounts.
    pub fn seed_reputation(&mut self, agent_id: &str, successes: u64, failures: u64) {
        for _ in 0..successes {
            self.reputation
                .record(agent_id, TransactionOutcome::Success, 0);
        }
        for _ in 0..failures {
            self.reputation
                .record(agent_id, TransactionOutcome::Failure, 0);
        }
    }

    // -- Listing management -------------------------------------------------

    fn listing_key(agent_id: &str, skill_name: &str) -> String {
        format!("{agent_id}/{skill_name}")
    }

    /// Add a skill listing to the marketplace.
    pub fn list_skill(&mut self, listing: SkillListing) -> Result<(), AgoraError> {
        // Check reputation gate
        let rep = self.reputation.score(&listing.agent_id);
        if rep.total_transactions > 0 && rep.score < self.config.min_listing_reputation {
            return Err(AgoraError::InvalidAmount(0));
        }

        let key = Self::listing_key(&listing.agent_id, &listing.skill_name);
        self.listings.insert(key, listing);
        Ok(())
    }

    /// Remove a skill listing.
    pub fn delist_skill(&mut self, agent_id: &str, skill_name: &str) -> Result<(), AgoraError> {
        let key = Self::listing_key(agent_id, skill_name);
        self.listings
            .remove(&key)
            .map(|_| ())
            .ok_or_else(|| AgoraError::ListingNotFound {
                agent_id: agent_id.to_string(),
                skill_name: skill_name.to_string(),
            })
    }

    /// Get a specific listing.
    pub fn get_listing(&self, agent_id: &str, skill_name: &str) -> Option<&SkillListing> {
        let key = Self::listing_key(agent_id, skill_name);
        self.listings.get(&key)
    }

    /// List all listings.
    pub fn all_listings(&self) -> Vec<&SkillListing> {
        self.listings.values().collect()
    }

    /// Count listings.
    pub fn listing_count(&self) -> usize {
        self.listings.len()
    }

    // -- Search -------------------------------------------------------------

    /// Search listings by criteria.
    pub fn search(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let mut results: Vec<SearchResult> = self
            .listings
            .values()
            .filter(|listing| {
                // Agent filter
                if let Some(ref aid) = query.agent_id
                    && listing.agent_id != *aid
                {
                    return false;
                }
                // Price filter
                if let Some(max) = query.max_price
                    && listing.price_credits > max
                {
                    return false;
                }
                // Success rate filter
                if let Some(min_sr) = query.min_success_rate
                    && listing.success_rate < min_sr
                {
                    return false;
                }
                // Tag filter (any-match). Previously declared but never applied —
                // now wired so listings must carry at least one queried tag.
                if !query.tags.is_empty()
                    && !query.tags.iter().any(|t| listing.tags.contains(t))
                {
                    return false;
                }
                // Capability filter (any-match). Symmetric with tags; listings
                // carry no capabilities today, so a non-empty query yields empty
                // (behavior-equivalent to legacy hardcoded `[]`).
                if !query.capabilities.is_empty()
                    && !query
                        .capabilities
                        .iter()
                        .any(|c| listing.capabilities.contains(c))
                {
                    return false;
                }
                true
            })
            .map(|listing| {
                let mut relevance = 1.0;

                // Text matching boost
                if let Some(ref text) = query.text {
                    let text_lower = text.to_lowercase();
                    if listing.skill_name.to_lowercase().contains(&text_lower) {
                        relevance += 2.0;
                    }
                    if listing.description.to_lowercase().contains(&text_lower) {
                        relevance += 1.0;
                    }
                }

                // Success rate boost
                relevance += listing.success_rate;

                // Execution volume boost (log scale)
                if listing.total_executions > 0 {
                    relevance += (listing.total_executions as f64).ln();
                }

                let rep = self.reputation.score(&listing.agent_id);

                SearchResult {
                    listing: listing.clone(),
                    relevance,
                    reputation: rep.score,
                }
            })
            .collect();

        // Sort by relevance descending
        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Apply limit
        if let Some(limit) = query.limit {
            results.truncate(limit);
        }

        results
    }

    // -- Purchasing ---------------------------------------------------------

    /// Execute a skill transaction with proper status transitions:
    /// `Pending → InProgress → Completed` on success, or `Pending → InProgress → Failed`
    /// if the buyer has insufficient funds or no registered wallet.
    ///
    /// Credits are only transferred after reaching `InProgress`. On failure the
    /// transaction is appended to the log as `Failed` so the history remains complete.
    pub fn execute_transaction(
        &mut self,
        buyer_id: &str,
        seller_id: &str,
        skill_name: &str,
    ) -> Result<SkillTransaction, AgoraError> {
        // Validate listing exists before touching wallets.
        let listing = self
            .get_listing(seller_id, skill_name)
            .ok_or_else(|| AgoraError::ListingNotFound {
                agent_id: seller_id.to_string(),
                skill_name: skill_name.to_string(),
            })?
            .clone();

        let price = listing.price_credits;
        let commission = (price as f64 * self.config.commission_rate).ceil() as i64;
        let seller_amount = price - commission;

        // Create transaction: Pending
        let mut tx = SkillTransaction::new(buyer_id, seller_id, skill_name, price);

        // Transition to InProgress
        tx.status = TransactionStatus::InProgress;

        // Attempt to debit buyer — on any error, mark Failed, log, and return the error.
        let spend_result = match self.wallets.get_mut(buyer_id) {
            None => Err(AgoraError::InsufficientCredits {
                available: 0,
                required: price,
            }),
            Some(w) => w.spend(price),
        };

        if let Err(e) = spend_result {
            tx.fail();
            self.transaction_log.append(tx);
            return Err(e);
        }

        // Call the settlement provider.  On failure: refund buyer, log Failed.
        let settlement_receipt = match self
            .settlement
            .settle(buyer_id, seller_id, price, skill_name)
        {
            Ok(r) => r,
            Err(e) => {
                // Rollback buyer debit.
                if let Some(w) = self.wallets.get_mut(buyer_id) {
                    let _ = w.earn(price);
                }
                tx.fail();
                self.transaction_log.append(tx);
                return Err(e);
            }
        };

        // Credit seller (auto-register wallet if first sale).
        let seller_wallet = self
            .wallets
            .entry(seller_id.to_string())
            .or_insert_with(|| AgentWallet::new(seller_id, 0));
        let _ = seller_wallet.earn(seller_amount);

        // Transition to Completed; attach settlement reference.
        tx.complete();
        tx.settlement_reference = settlement_receipt.reference;
        self.transaction_log.append(tx.clone());

        // Update reputation.
        self.reputation
            .record(seller_id, TransactionOutcome::Success, price);
        self.reputation
            .record(buyer_id, TransactionOutcome::Success, price);

        // Update listing execution stats.
        let key = Self::listing_key(seller_id, skill_name);
        if let Some(l) = self.listings.get_mut(&key) {
            l.total_executions += 1;
        }

        Ok(tx)
    }

    /// Convenience wrapper — delegates to [`execute_transaction`].
    pub fn purchase(
        &mut self,
        buyer_id: &str,
        seller_id: &str,
        skill_name: &str,
    ) -> Result<SkillTransaction, AgoraError> {
        self.execute_transaction(buyer_id, seller_id, skill_name)
    }

    /// Query the transaction log.
    ///
    /// Results are returned most-recent-first. Use [`TransactionFilter`] to
    /// narrow by agent, skill, time range, or status.
    pub fn list_transactions(&self, filter: &TransactionFilter) -> Vec<&SkillTransaction> {
        self.transaction_log.query(filter)
    }

    // -- Direct wallet operations (web4 P0-2) -------------------------------

    /// Transfer `amount` credits directly from one agent to another.
    ///
    /// **Atomicity guarantee:** all preconditions (both wallets exist, sender
    /// has sufficient funds, amount is positive) are validated *before* any
    /// balance is mutated. The debit and credit then apply together within
    /// this single `&mut self` call — there is no intermediate state in which
    /// the sender is debited but the receiver is not credited. A debit failure
    /// returns `Err` with zero mutation; a credit failure (which cannot occur
    /// for a positive amount on a validated wallet) rolls the debit back before
    /// returning.
    ///
    /// Returns `Err(InvalidAmount)` for non-positive amounts,
    /// `Err(WalletNotFound)` if either party has no registered wallet, and
    /// `Err(InsufficientCredits)` if the sender cannot cover the transfer.
    pub fn wallet_pay(
        &mut self,
        from: &str,
        to: &str,
        amount: i64,
        _memo: Option<&str>,
    ) -> Result<(), AgoraError> {
        // --- Validate-before-mutate (atomicity) ---------------------------
        if amount <= 0 {
            return Err(AgoraError::InvalidAmount(amount));
        }
        if from == to {
            return Err(AgoraError::InvalidAmount(amount));
        }
        // Both wallets must exist before we touch any balance.
        if !self.wallets.contains_key(from) {
            return Err(AgoraError::WalletNotFound(from.to_string()));
        }
        if !self.wallets.contains_key(to) {
            return Err(AgoraError::WalletNotFound(to.to_string()));
        }
        // Sender must cover the transfer (checked here so we never half-apply).
        let sender_balance = self
            .wallets
            .get(from)
            .map(|w| w.balance)
            .expect("sender wallet existence checked above");
        if sender_balance < amount {
            return Err(AgoraError::InsufficientCredits {
                available: sender_balance,
                required: amount,
            });
        }

        // --- Apply (preconditions hold; both sides settle together) -------
        // Debit sender. Cannot fail: positive amount + balance >= amount.
        self.wallets
            .get_mut(from)
            .expect("sender wallet existence checked above")
            .spend(amount)?;
        // Credit receiver. Roll the debit back if it somehow fails.
        if let Some(w) = self.wallets.get_mut(to) {
            if let Err(e) = w.earn(amount) {
                // Compensating action: undo the debit to preserve invariant.
                let _ = self
                    .wallets
                    .get_mut(from)
                    .expect("sender wallet existence checked above")
                    .earn(amount);
                return Err(e);
            }
        }
        Ok(())
    }

    /// Return an agent's transaction history, most-recent-first.
    ///
    /// Matches transactions where the agent is either buyer or seller. `limit`
    /// caps the number of returned entries (0 or `None` means no cap).
    pub fn wallet_history(&self, agent_id: &str, limit: Option<usize>) -> Vec<SkillTransaction> {
        let filter = TransactionFilter {
            agent_id: Some(agent_id.to_string()),
            skill_name: None,
            after: None,
            before: None,
            status: None,
            limit,
        };
        self.transaction_log
            .query(&filter)
            .into_iter()
            .cloned()
            .collect()
    }


    // -- Two-phase settlement saga ------------------------------------------

    /// Phase 1 — debit buyer and open an in-flight reservation.
    ///
    /// Returns the transaction UUID.  Call [`commit_transaction`] on success
    /// or [`abort_transaction`] to roll back the buyer debit.
    pub fn begin_transaction(
        &mut self,
        buyer_id: &str,
        listing: &SkillListing,
    ) -> Result<Uuid, AgoraError> {
        let price = listing.price_credits;
        if price <= 0 {
            return Err(AgoraError::InvalidAmount(price));
        }

        match self.wallets.get_mut(buyer_id) {
            None => {
                return Err(AgoraError::InsufficientCredits {
                    available: 0,
                    required: price,
                });
            }
            Some(w) => w.spend(price)?,
        }

        let tx = SkillTransaction::new(buyer_id, &listing.agent_id, &listing.skill_name, price);
        let id = tx.id;
        self.pending.insert(id, tx);
        Ok(id)
    }

    /// Abort a pending transaction and refund the buyer's credits.
    pub fn abort_transaction(&mut self, tx_id: Uuid) -> Result<(), AgoraError> {
        let tx = self
            .pending
            .remove(&tx_id)
            .ok_or(AgoraError::TransactionNotFound(tx_id))?;

        if let Some(w) = self.wallets.get_mut(&tx.buyer_agent_id) {
            let _ = w.earn(tx.credits_transferred);
        }

        Ok(())
    }

    /// Phase 3b — finalize a pending transaction.
    ///
    /// Credits the seller (net of commission), attaches the optional on-chain
    /// `reference`, marks the transaction `Completed`, updates reputation and
    /// listing stats, and appends the record to the transaction log.
    pub fn commit_transaction(
        &mut self,
        tx_id: Uuid,
        reference: Option<String>,
    ) -> Result<SkillTransaction, AgoraError> {
        let mut tx = self
            .pending
            .remove(&tx_id)
            .ok_or(AgoraError::TransactionNotFound(tx_id))?;

        let price = tx.credits_transferred;
        let commission = (price as f64 * self.config.commission_rate).ceil() as i64;
        let seller_amount = price - commission;

        // Credit seller (auto-register wallet on first sale).
        let seller_wallet = self
            .wallets
            .entry(tx.seller_agent_id.clone())
            .or_insert_with(|| AgentWallet::new(&tx.seller_agent_id, 0));
        let _ = seller_wallet.earn(seller_amount);

        tx.complete();
        tx.settlement_reference = reference;
        self.transaction_log.append(tx.clone());

        self.reputation
            .record(&tx.seller_agent_id, TransactionOutcome::Success, price);
        self.reputation
            .record(&tx.buyer_agent_id, TransactionOutcome::Success, price);

        let key = Self::listing_key(&tx.seller_agent_id, &tx.skill_name);
        if let Some(l) = self.listings.get_mut(&key) {
            l.total_executions += 1;
        }

        Ok(tx)
    }

    // -- Reputation ---------------------------------------------------------

    /// Get reputation score for an agent.
    pub fn reputation(&self, agent_id: &str) -> ReputationScore {
        self.reputation.score(agent_id)
    }

    // -- Disputes -----------------------------------------------------------

    /// File a dispute for a transaction.
    pub fn file_dispute(
        &mut self,
        transaction_id: Uuid,
        filed_by: &str,
        against: &str,
        reason: &str,
    ) -> Result<String, AgoraError> {
        let dispute_id = self
            .disputes
            .file(transaction_id, filed_by, against, reason)?;
        self.reputation
            .record(against, TransactionOutcome::Disputed, 0);
        Ok(dispute_id)
    }

    /// Get dispute by ID.
    pub fn get_dispute(&self, dispute_id: &str) -> Option<&Dispute> {
        self.disputes.get(dispute_id)
    }

    /// Count open disputes.
    pub fn open_disputes(&self) -> usize {
        self.disputes.open_count()
    }

    // -- Categories ---------------------------------------------------------

    /// Get the category index.
    pub fn categories(&self) -> &CategoryIndex {
        &self.categories
    }

    // -- Stats --------------------------------------------------------------

    /// Total transactions logged (includes both Completed and Failed).
    pub fn transaction_count(&self) -> usize {
        self.transaction_log.count()
    }

    /// Total credits transacted (sum across all logged entries).
    pub fn total_volume(&self) -> i64 {
        self.transaction_log.total_volume()
    }

    // -- Ported consumer methods (web4 P0-1b cut-2) -------------------------
    // Behavior-equivalent ports of the zeus-marketplace facade methods the
    // economy/pantheon/fleet handlers consume, adapted to agora's sync engine.

    /// Snapshot of every agent's balance, keyed by agent_id.
    ///
    /// Port of `zeus-marketplace::Marketplace::all_balances`. Agora stores
    /// balances as `i64`; callers map to their wire type at the boundary.
    pub fn all_balances(&self) -> HashMap<String, i64> {
        self.wallets
            .iter()
            .map(|(id, w)| (id.clone(), w.balance))
            .collect()
    }

    /// Reputation score for every agent agora has recorded.
    ///
    /// Port of `zeus-marketplace::Marketplace::all_reputations`, surfaced as
    /// agora's native `ReputationScore` keyed by agent_id.
    pub fn all_reputations(&self) -> Vec<(String, ReputationScore)> {
        self.reputation
            .agent_ids()
            .into_iter()
            .map(|id| {
                let score = self.reputation.score(&id);
                (id, score)
            })
            .collect()
    }

    /// Increment a listing's execution counter (download/invocation tally).
    ///
    /// Port of `zeus-marketplace::Marketplace::record_download`, keyed by
    /// agora's `{agent_id}/{skill_name}`. Returns the new total, or a
    /// `ListingNotFound` error if the listing is absent.
    pub fn record_download(
        &mut self,
        agent_id: &str,
        skill_name: &str,
    ) -> Result<u64, AgoraError> {
        let key = Self::listing_key(agent_id, skill_name);
        match self.listings.get_mut(&key) {
            Some(listing) => {
                listing.total_executions += 1;
                Ok(listing.total_executions)
            }
            None => Err(AgoraError::ListingNotFound {
                agent_id: agent_id.to_string(),
                skill_name: skill_name.to_string(),
            }),
        }
    }

    /// Aggregate marketplace statistics.
    ///
    /// Port of `zeus-marketplace::Marketplace::stats`. `active_listings`
    /// mirrors `total_listings` in agora's model (no soft-delist flag).
    pub fn stats(&self) -> MarketplaceStats {
        let total_listings = self.listings.len();
        MarketplaceStats {
            total_listings,
            active_listings: total_listings,
            total_trades: self.transaction_log.count(),
            completed_trades: self.transaction_log.count(),
            total_token_supply: self.wallets.values().map(|w| w.balance.max(0) as u64).sum(),
            total_agents: self.reputation.agent_ids().len(),
        }
    }

    /// Find listings whose name or description matches `query` (case-insensitive).
    ///
    /// Adapter for `zeus-marketplace::Marketplace::search_by_name`, expressed
    /// over agora's `search()` text axis.
    pub fn search_by_name(&self, query: &str) -> Vec<SkillListing> {
        self.search(&SearchQuery {
            text: Some(query.to_string()),
            ..Default::default()
        })
        .into_iter()
        .map(|r| r.listing)
        .collect()
    }

    /// Find listings published by `agent_id`.
    ///
    /// Adapter for `zeus-marketplace::Marketplace::search_by_publisher`,
    /// expressed over agora's `search()` agent axis.
    pub fn search_by_publisher(&self, agent_id: &str) -> Vec<SkillListing> {
        self.search(&SearchQuery {
            agent_id: Some(agent_id.to_string()),
            ..Default::default()
        })
        .into_iter()
        .map(|r| r.listing)
        .collect()
    }
}

/// Aggregate marketplace statistics (web4 P0-1b cut-2 port).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MarketplaceStats {
    pub total_listings: usize,
    pub active_listings: usize,
    pub total_trades: usize,
    pub completed_trades: usize,
    pub total_token_supply: u64,
    pub total_agents: usize,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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

    // -- MarketplaceConfig --------------------------------------------------

    #[test]
    fn test_config_defaults() {
        let cfg = MarketplaceConfig::default();
        assert_eq!(cfg.reputation_window, 100);
        assert_eq!(cfg.max_open_disputes, 5);
        assert!((cfg.commission_rate - 0.05).abs() < f64::EPSILON);
    }

    // -- ReputationTracker --------------------------------------------------

    #[test]
    fn test_reputation_default_score() {
        let tracker = ReputationTracker::new(100);
        let score = tracker.score("unknown");
        assert!((score.score - 0.5).abs() < f64::EPSILON);
        assert_eq!(score.total_transactions, 0);
    }

    #[test]
    fn test_reputation_all_successes() {
        let mut tracker = ReputationTracker::new(100);
        for _ in 0..10 {
            tracker.record("agent-1", TransactionOutcome::Success, 10);
        }
        let score = tracker.score("agent-1");
        assert!((score.score - 1.0).abs() < f64::EPSILON);
        assert_eq!(score.successes, 10);
        assert_eq!(score.total_credits, 100);
    }

    #[test]
    fn test_reputation_mixed() {
        let mut tracker = ReputationTracker::new(100);
        tracker.record("a", TransactionOutcome::Success, 10);
        tracker.record("a", TransactionOutcome::Failure, 10);
        let score = tracker.score("a");
        assert!((score.score - 0.5).abs() < f64::EPSILON);
        assert_eq!(score.successes, 1);
        assert_eq!(score.failures, 1);
    }

    #[test]
    fn test_reputation_window_eviction() {
        let mut tracker = ReputationTracker::new(3);
        // Record 3 successes then 3 failures — window keeps last 3
        for _ in 0..3 {
            tracker.record("a", TransactionOutcome::Success, 10);
        }
        for _ in 0..3 {
            tracker.record("a", TransactionOutcome::Failure, 10);
        }
        let score = tracker.score("a");
        assert_eq!(score.total_transactions, 3);
        assert_eq!(score.failures, 3);
        assert!((score.score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reputation_tracked_agents() {
        let mut tracker = ReputationTracker::new(100);
        tracker.record("a", TransactionOutcome::Success, 10);
        tracker.record("b", TransactionOutcome::Success, 10);
        let agents = tracker.tracked_agents();
        assert_eq!(agents.len(), 2);
    }

    #[test]
    fn test_reputation_clear() {
        let mut tracker = ReputationTracker::new(100);
        tracker.record("a", TransactionOutcome::Success, 10);
        tracker.clear("a");
        let score = tracker.score("a");
        assert_eq!(score.total_transactions, 0);
    }

    #[test]
    fn test_reputation_disputed_partial_score() {
        let mut tracker = ReputationTracker::new(100);
        tracker.record("a", TransactionOutcome::Disputed, 10);
        let score = tracker.score("a");
        // Disputed = 0.3 weight → 0.3/1 = 0.3
        assert!((score.score - 0.3).abs() < f64::EPSILON);
        assert_eq!(score.disputes, 1);
    }

    #[test]
    fn test_transaction_outcome_from_status() {
        assert_eq!(
            TransactionOutcome::from(TransactionStatus::Completed),
            TransactionOutcome::Success
        );
        assert_eq!(
            TransactionOutcome::from(TransactionStatus::Failed),
            TransactionOutcome::Failure
        );
        assert_eq!(
            TransactionOutcome::from(TransactionStatus::Refunded),
            TransactionOutcome::Refunded
        );
    }

    // -- DisputeManager -----------------------------------------------------

    #[test]
    fn test_dispute_file_and_get() {
        let mut mgr = DisputeManager::new(5, 86400);
        let tx_id = Uuid::new_v4();
        let id = mgr.file(tx_id, "buyer", "seller", "bad service").unwrap();
        let dispute = mgr.get(&id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::Open);
        assert_eq!(dispute.filed_by, "buyer");
        assert_eq!(dispute.against, "seller");
    }

    #[test]
    fn test_dispute_add_evidence() {
        let mut mgr = DisputeManager::new(5, 86400);
        let id = mgr.file(Uuid::new_v4(), "b", "s", "reason").unwrap();
        mgr.add_evidence(&id, "screenshot.png").unwrap();
        let dispute = mgr.get(&id).unwrap();
        assert_eq!(dispute.evidence.len(), 1);
        assert_eq!(dispute.status, DisputeStatus::UnderReview);
    }

    #[test]
    fn test_dispute_resolve_buyer() {
        let mut mgr = DisputeManager::new(5, 86400);
        let id = mgr.file(Uuid::new_v4(), "b", "s", "reason").unwrap();
        mgr.resolve(&id, true, "buyer was right").unwrap();
        let dispute = mgr.get(&id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::ResolvedBuyer);
        assert!(dispute.resolved_at.is_some());
    }

    #[test]
    fn test_dispute_resolve_seller() {
        let mut mgr = DisputeManager::new(5, 86400);
        let id = mgr.file(Uuid::new_v4(), "b", "s", "reason").unwrap();
        mgr.resolve(&id, false, "seller delivered").unwrap();
        let dispute = mgr.get(&id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::ResolvedSeller);
    }

    #[test]
    fn test_dispute_dismiss() {
        let mut mgr = DisputeManager::new(5, 86400);
        let id = mgr.file(Uuid::new_v4(), "b", "s", "reason").unwrap();
        mgr.dismiss(&id, "no merit").unwrap();
        let dispute = mgr.get(&id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::Dismissed);
    }

    #[test]
    fn test_dispute_max_open_limit() {
        let mut mgr = DisputeManager::new(2, 86400);
        mgr.file(Uuid::new_v4(), "b", "s1", "r1").unwrap();
        mgr.file(Uuid::new_v4(), "b", "s2", "r2").unwrap();
        let err = mgr.file(Uuid::new_v4(), "b", "s3", "r3");
        assert!(err.is_err());
    }

    #[test]
    fn test_dispute_list_for_agent() {
        let mut mgr = DisputeManager::new(5, 86400);
        mgr.file(Uuid::new_v4(), "buyer", "seller1", "r1").unwrap();
        mgr.file(Uuid::new_v4(), "buyer", "seller2", "r2").unwrap();
        mgr.file(Uuid::new_v4(), "other", "buyer", "r3").unwrap();
        let disputes = mgr.list_for_agent("buyer");
        assert_eq!(disputes.len(), 3); // 2 filed by + 1 against
    }

    #[test]
    fn test_dispute_open_count() {
        let mut mgr = DisputeManager::new(5, 86400);
        let id1 = mgr.file(Uuid::new_v4(), "b", "s", "r").unwrap();
        mgr.file(Uuid::new_v4(), "b2", "s", "r").unwrap();
        assert_eq!(mgr.open_count(), 2);
        mgr.resolve(&id1, true, "done").unwrap();
        assert_eq!(mgr.open_count(), 1);
    }

    // -- CategoryIndex ------------------------------------------------------

    #[test]
    fn test_category_defaults() {
        let idx = CategoryIndex::with_defaults();
        assert_eq!(idx.count(), 4);
        assert!(idx.get("Code").is_some());
        assert!(idx.get("Data").is_some());
        assert!(idx.get("Language").is_some());
        assert!(idx.get("Security").is_some());
    }

    #[test]
    fn test_category_match_tags() {
        let idx = CategoryIndex::with_defaults();
        let matches = idx.match_tags(&["code".to_string()]);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "Code");
    }

    #[test]
    fn test_category_match_multiple_tags() {
        let idx = CategoryIndex::with_defaults();
        let matches = idx.match_tags(&["code".to_string(), "security".to_string()]);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_category_no_match() {
        let idx = CategoryIndex::with_defaults();
        let matches = idx.match_tags(&["nonexistent".to_string()]);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_category_add_custom() {
        let mut idx = CategoryIndex::new();
        idx.add(Category {
            name: "Custom".to_string(),
            description: "Custom category".to_string(),
            tags: vec!["custom".into()],
        });
        assert_eq!(idx.count(), 1);
        assert!(idx.get("Custom").is_some());
    }

    // -- Marketplace --------------------------------------------------------

    #[test]
    fn test_marketplace_new() {
        let mp = Marketplace::with_defaults();
        assert_eq!(mp.listing_count(), 0);
        assert_eq!(mp.transaction_count(), 0);
        assert_eq!(mp.open_disputes(), 0);
    }

    #[test]
    fn test_marketplace_register_wallet() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("agent-1", 1000);
        assert_eq!(mp.balance("agent-1"), Some(1000));
        assert_eq!(mp.balance("unknown"), None);
    }

    #[test]
    fn test_marketplace_list_skill() {
        let mut mp = Marketplace::with_defaults();
        let listing = test_listing("agent-1", "code_review", 25);
        mp.list_skill(listing).unwrap();
        assert_eq!(mp.listing_count(), 1);
        assert!(mp.get_listing("agent-1", "code_review").is_some());
    }

    #[test]
    fn test_marketplace_delist_skill() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "s", 10)).unwrap();
        mp.delist_skill("a", "s").unwrap();
        assert_eq!(mp.listing_count(), 0);
    }

    #[test]
    fn test_marketplace_delist_nonexistent() {
        let mut mp = Marketplace::with_defaults();
        let err = mp.delist_skill("a", "s").unwrap_err();
        assert!(matches!(err, AgoraError::ListingNotFound { .. }));
    }

    #[test]
    fn test_marketplace_purchase_flow() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "analyze", 20))
            .unwrap();

        let tx = mp.purchase("buyer", "seller", "analyze").unwrap();
        assert_eq!(tx.status, TransactionStatus::Completed);
        assert_eq!(tx.credits_transferred, 20);

        // Commission: 5% of 20 = 1
        assert_eq!(mp.balance("buyer"), Some(80));
        assert_eq!(mp.balance("seller"), Some(19)); // 20 - 1 commission

        assert_eq!(mp.transaction_count(), 1);
        assert_eq!(mp.total_volume(), 20);
    }

    #[test]
    fn test_marketplace_purchase_insufficient_funds() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 5);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "expensive", 100))
            .unwrap();

        let err = mp.purchase("buyer", "seller", "expensive").unwrap_err();
        assert!(matches!(err, AgoraError::InsufficientCredits { .. }));
    }

    #[test]
    fn test_marketplace_purchase_nonexistent_listing() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);

        let err = mp.purchase("buyer", "seller", "ghost").unwrap_err();
        assert!(matches!(err, AgoraError::ListingNotFound { .. }));
    }

    #[test]
    fn test_marketplace_search_all() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "code_review", 10)).unwrap();
        mp.list_skill(test_listing("b", "translate", 20)).unwrap();

        let results = mp.search(&SearchQuery::default());
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_marketplace_search_by_text() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "code_review", 10)).unwrap();
        mp.list_skill(test_listing("b", "translate", 20)).unwrap();

        let results = mp.search(&SearchQuery {
            text: Some("code".to_string()),
            ..Default::default()
        });
        // Both returned but code_review ranked higher
        assert!(results[0].listing.skill_name == "code_review");
    }

    #[test]
    fn test_marketplace_search_by_price() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "cheap", 5)).unwrap();
        mp.list_skill(test_listing("b", "expensive", 100)).unwrap();

        let results = mp.search(&SearchQuery {
            max_price: Some(10),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].listing.skill_name, "cheap");
    }

    #[test]
    fn test_marketplace_search_by_agent() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "s1", 10)).unwrap();
        mp.list_skill(test_listing("b", "s2", 10)).unwrap();

        let results = mp.search(&SearchQuery {
            agent_id: Some("a".to_string()),
            ..Default::default()
        });
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_marketplace_search_limit() {
        let mut mp = Marketplace::with_defaults();
        for i in 0..10 {
            mp.list_skill(test_listing("a", &format!("skill-{i}"), 10))
                .unwrap();
        }
        let results = mp.search(&SearchQuery {
            limit: Some(3),
            ..Default::default()
        });
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_marketplace_reputation_after_purchase() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();
        mp.purchase("buyer", "seller", "skill").unwrap();

        let rep = mp.reputation("seller");
        assert!((rep.score - 1.0).abs() < f64::EPSILON);
        assert_eq!(rep.successes, 1);
    }

    #[test]
    fn test_marketplace_dispute_flow() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();
        let tx = mp.purchase("buyer", "seller", "skill").unwrap();

        let dispute_id = mp
            .file_dispute(tx.id, "buyer", "seller", "bad quality")
            .unwrap();
        assert_eq!(mp.open_disputes(), 1);

        let dispute = mp.get_dispute(&dispute_id).unwrap();
        assert_eq!(dispute.status, DisputeStatus::Open);
    }

    #[test]
    fn test_marketplace_all_listings() {
        let mut mp = Marketplace::with_defaults();
        mp.list_skill(test_listing("a", "s1", 10)).unwrap();
        mp.list_skill(test_listing("b", "s2", 20)).unwrap();
        assert_eq!(mp.all_listings().len(), 2);
    }

    #[test]
    fn test_marketplace_purchase_updates_listing_stats() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();
        mp.purchase("buyer", "seller", "skill").unwrap();

        let listing = mp.get_listing("seller", "skill").unwrap();
        assert_eq!(listing.total_executions, 1);
    }

    // -- execute_transaction + TransactionLog + list_transactions ---------------

    #[test]
    fn test_execute_transaction_status_flow() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "analyze", 20))
            .unwrap();

        let tx = mp
            .execute_transaction("buyer", "seller", "analyze")
            .unwrap();

        // Final status must be Completed with a timestamp.
        assert_eq!(tx.status, TransactionStatus::Completed);
        assert!(tx.completed_at.is_some());
        assert_eq!(tx.credits_transferred, 20);

        // Should appear in log.
        assert_eq!(mp.transaction_count(), 1);
    }

    #[test]
    fn test_execute_transaction_insufficient_funds_logs_failed() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("broke_buyer", 5);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "expensive", 50))
            .unwrap();

        let err = mp
            .execute_transaction("broke_buyer", "seller", "expensive")
            .unwrap_err();
        assert!(matches!(err, AgoraError::InsufficientCredits { .. }));

        // Failed transaction is still logged.
        assert_eq!(mp.transaction_count(), 1);
        let log = mp.list_transactions(&TransactionFilter::default());
        assert_eq!(log[0].status, TransactionStatus::Failed);

        // Buyer balance unchanged.
        assert_eq!(mp.balance("broke_buyer"), Some(5));
    }

    #[test]
    fn test_execute_transaction_no_wallet_logs_failed() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();

        // Buyer never registered a wallet.
        let err = mp
            .execute_transaction("ghost_buyer", "seller", "skill")
            .unwrap_err();
        assert!(matches!(
            err,
            AgoraError::InsufficientCredits { available: 0, .. }
        ));
        assert_eq!(mp.transaction_count(), 1);
        let log = mp.list_transactions(&TransactionFilter::default());
        assert_eq!(log[0].status, TransactionStatus::Failed);
    }

    #[test]
    fn test_execute_transaction_sequential_drains_balance() {
        // Two sequential purchases from the same buyer; the second should fail
        // once funds are exhausted — exercising the "concurrent drain" case.
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 70);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 60)).unwrap();

        // First purchase succeeds.
        let tx1 = mp.execute_transaction("buyer", "seller", "skill").unwrap();
        assert_eq!(tx1.status, TransactionStatus::Completed);
        assert_eq!(mp.balance("buyer"), Some(10)); // 70 - 60

        // Second purchase fails — only 10 credits left.
        let err = mp
            .execute_transaction("buyer", "seller", "skill")
            .unwrap_err();
        assert!(matches!(err, AgoraError::InsufficientCredits { .. }));

        // Log has both transactions: one Completed, one Failed.
        assert_eq!(mp.transaction_count(), 2);
        let completed = mp.list_transactions(&TransactionFilter {
            status: Some(TransactionStatus::Completed),
            ..Default::default()
        });
        let failed = mp.list_transactions(&TransactionFilter {
            status: Some(TransactionStatus::Failed),
            ..Default::default()
        });
        assert_eq!(completed.len(), 1);
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn test_list_transactions_filter_by_agent() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("alice", 200);
        mp.register_wallet("bob", 200);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();

        mp.execute_transaction("alice", "seller", "skill").unwrap();
        mp.execute_transaction("alice", "seller", "skill").unwrap();
        mp.execute_transaction("bob", "seller", "skill").unwrap();

        let alice_txns = mp.list_transactions(&TransactionFilter {
            agent_id: Some("alice".to_string()),
            ..Default::default()
        });
        assert_eq!(alice_txns.len(), 2);

        let bob_txns = mp.list_transactions(&TransactionFilter {
            agent_id: Some("bob".to_string()),
            ..Default::default()
        });
        assert_eq!(bob_txns.len(), 1);
    }

    #[test]
    fn test_list_transactions_filter_by_skill() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 200);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill_a", 10))
            .unwrap();
        mp.list_skill(test_listing("seller", "skill_b", 10))
            .unwrap();

        mp.execute_transaction("buyer", "seller", "skill_a")
            .unwrap();
        mp.execute_transaction("buyer", "seller", "skill_b")
            .unwrap();
        mp.execute_transaction("buyer", "seller", "skill_a")
            .unwrap();

        let skill_a = mp.list_transactions(&TransactionFilter {
            skill_name: Some("skill_a".to_string()),
            ..Default::default()
        });
        assert_eq!(skill_a.len(), 2);

        let skill_b = mp.list_transactions(&TransactionFilter {
            skill_name: Some("skill_b".to_string()),
            ..Default::default()
        });
        assert_eq!(skill_b.len(), 1);
    }

    #[test]
    fn test_list_transactions_filter_limit() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 1000);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 5)).unwrap();

        for _ in 0..10 {
            mp.execute_transaction("buyer", "seller", "skill").unwrap();
        }

        let limited = mp.list_transactions(&TransactionFilter {
            limit: Some(3),
            ..Default::default()
        });
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn test_transaction_log_most_recent_first() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 1000);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 5)).unwrap();

        mp.execute_transaction("buyer", "seller", "skill").unwrap();
        mp.execute_transaction("buyer", "seller", "skill").unwrap();

        let results = mp.list_transactions(&TransactionFilter::default());
        assert_eq!(results.len(), 2);
        // Most recent should be first (created_at descending).
        assert!(results[0].created_at >= results[1].created_at);
    }

    // -- SettlementProvider + X402Settlement ------------------------------------

    /// Mock that always succeeds with a custom reference tag.
    struct MockSettlement {
        reference: String,
    }
    impl SettlementProvider for MockSettlement {
        fn settle(
            &self,
            _b: &str,
            _s: &str,
            _a: i64,
            _sk: &str,
        ) -> Result<SettlementReceipt, AgoraError> {
            Ok(SettlementReceipt {
                method: "mock".to_string(),
                reference: Some(self.reference.clone()),
                on_chain_amount: 42,
            })
        }
    }

    /// Mock that always fails.
    struct FailingSettlement;
    impl SettlementProvider for FailingSettlement {
        fn settle(
            &self,
            _b: &str,
            _s: &str,
            _a: i64,
            _sk: &str,
        ) -> Result<SettlementReceipt, AgoraError> {
            Err(AgoraError::SettlementFailed("mock failure".to_string()))
        }
    }

    #[test]
    fn test_in_memory_settlement_is_default() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();

        let tx = mp.execute_transaction("buyer", "seller", "skill").unwrap();
        assert_eq!(tx.status, TransactionStatus::Completed);
        // In-memory settlement leaves no on-chain reference.
        assert!(tx.settlement_reference.is_none());
    }

    // ── #81a Phase 3: canonical-settlement projection ──────────────────────

    #[test]
    fn canonical_settlement_disabled_by_default() {
        // No env flag ⇒ inert no-op, even with a valid db path.
        let tmp = tempfile::tempdir().unwrap();
        let cs = CanonicalSettlement::from_env(tmp.path().join("economy.db"), 0.05);
        assert!(
            !cs.is_enabled(),
            "must be disabled unless ZEUS_UNIFY_MARKETPLACE is set"
        );
        // settle() is a pure no-op returning the in-memory receipt.
        let r = cs.settle("buyer", "seller", 100, "skill").unwrap();
        assert_eq!(r.method, "in-memory");
        assert!(r.reference.is_none());
    }

    #[test]
    fn canonical_settlement_disabled_construct_is_inert() {
        let cs = CanonicalSettlement::disabled();
        assert!(!cs.is_enabled());
        let r = cs.settle("b", "s", 50, "x").unwrap();
        assert_eq!(r.method, "in-memory");
    }

    #[test]
    fn canonical_settlement_mirrors_when_enabled() {
        // Build an enabled provider directly on a real economy ledger (bypass
        // env so the test is hermetic and parallel-safe).
        let tmp = tempfile::tempdir().unwrap();
        let ledger = zeus_economy::TokenLedger::new(tmp.path().join("economy.db")).unwrap();
        // Seed the buyer on the canonical ledger.
        ledger
            .mint(
                "buyer",
                1000,
                zeus_economy::TransactionReason::HumanTaskInjection,
                "seed",
            )
            .unwrap();
        let cs = CanonicalSettlement {
            enabled: true,
            fee_collector: "zeus-treasury".to_string(),
            ledger: Some(std::sync::Arc::new(ledger)),
            commission_rate: 0.10,
        };
        assert!(cs.is_enabled());

        let r = cs.settle("buyer", "seller", 100, "summarize").unwrap();
        assert_eq!(r.method, "canonical-economy");
        assert!(r.reference.is_some());

        // Reopen the ledger and assert canonical balances moved: buyer -100,
        // seller +90 (10% fee), treasury +10.
        let ledger2 = zeus_economy::TokenLedger::new(tmp.path().join("economy.db")).unwrap();
        assert_eq!(ledger2.balance("buyer").unwrap(), 900);
        assert_eq!(ledger2.balance("seller").unwrap(), 90);
        assert_eq!(ledger2.balance("zeus-treasury").unwrap(), 10);
    }

    #[test]
    fn canonical_settlement_mirror_failure_is_non_fatal() {
        // Enabled, but the buyer has no canonical funds ⇒ settle_trade errors.
        // The provider must swallow the error (mirror-not-gate) and return Ok.
        let tmp = tempfile::tempdir().unwrap();
        let ledger = zeus_economy::TokenLedger::new(tmp.path().join("economy.db")).unwrap();
        let cs = CanonicalSettlement {
            enabled: true,
            fee_collector: "zeus-treasury".to_string(),
            ledger: Some(std::sync::Arc::new(ledger)),
            commission_rate: 0.05,
        };
        // Buyer unfunded on canonical ledger → settle_trade fails internally.
        let r = cs.settle("broke-buyer", "seller", 100, "skill").unwrap();
        // Non-fatal: returns Ok with the in-memory-fallback receipt.
        assert_eq!(r.method, "in-memory-fallback");
        assert!(r.reference.is_none());
    }

    #[test]
    fn test_custom_settlement_receipt_attached() {
        let settlement = Box::new(MockSettlement {
            reference: "mock-ref-abc123".to_string(),
        });
        let mut mp = Marketplace::with_settlement(MarketplaceConfig::default(), settlement);
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 10)).unwrap();

        let tx = mp.execute_transaction("buyer", "seller", "skill").unwrap();
        assert_eq!(tx.status, TransactionStatus::Completed);
        assert_eq!(tx.settlement_reference.as_deref(), Some("mock-ref-abc123"));
    }

    #[test]
    fn test_settlement_failure_refunds_buyer_and_logs_failed() {
        let mut mp =
            Marketplace::with_settlement(MarketplaceConfig::default(), Box::new(FailingSettlement));
        mp.register_wallet("buyer", 100);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "skill", 30)).unwrap();

        let err = mp
            .execute_transaction("buyer", "seller", "skill")
            .unwrap_err();
        assert!(matches!(err, AgoraError::SettlementFailed(_)));

        // Buyer balance must be restored (refund on settlement failure).
        assert_eq!(mp.balance("buyer"), Some(100));
        // Seller receives nothing.
        assert_eq!(mp.balance("seller"), Some(0));
        // Failed tx is still in the log.
        let log = mp.list_transactions(&TransactionFilter::default());
        assert_eq!(log[0].status, TransactionStatus::Failed);
    }

    #[test]
    fn test_x402_settlement_micro_usdc_conversion() {
        let s = X402Settlement::new(X402Config::default(), "https://seller.example", 100);
        assert_eq!(s.to_micro_usdc(100), 1_000_000); // 100 credits = 1 USDC
        assert_eq!(s.to_micro_usdc(1), 10_000); // 1 credit = 0.01 USDC
        assert_eq!(s.to_micro_usdc(0), 0);
    }

    #[test]
    fn test_x402_settlement_happy_path() {
        let config = X402Config {
            max_amount: 10_000_000, // 10 USDC
            allowed_networks: vec!["solana-devnet".to_string()],
            allowed_tokens: vec!["EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()],
        };
        let s = X402Settlement::new(config, "https://seller.example/api", 100);

        let receipt = s.settle("buyer-1", "seller-1", 50, "code_review").unwrap();
        assert_eq!(receipt.method, "x402");
        assert!(receipt.reference.is_some());
        let reference = receipt.reference.unwrap();
        assert!(reference.contains("buyer-1"));
        assert!(reference.contains("code_review"));
        assert!(reference.contains("500000µUSDC")); // 50 credits × 10000 µUSDC/credit
        assert_eq!(receipt.on_chain_amount, 500_000);
    }

    #[test]
    fn test_x402_settlement_exceeds_cap() {
        let config = X402Config {
            max_amount: 100, // only 100 µUSDC allowed
            allowed_networks: vec!["solana-devnet".to_string()],
            allowed_tokens: vec!["USDC".to_string()],
        };
        let s = X402Settlement::new(config, "https://seller.example", 100);

        // 10 credits = 100_000 µUSDC → exceeds cap of 100
        let err = s.settle("buyer", "seller", 10, "skill").unwrap_err();
        assert!(matches!(err, AgoraError::SettlementFailed(_)));
    }

    #[test]
    fn test_marketplace_with_x402_settlement() {
        // Wire an X402Settlement into the marketplace and verify the reference
        // is stored on the completed transaction.
        let config = X402Config {
            max_amount: 100_000_000,
            allowed_networks: vec!["solana-devnet".to_string()],
            allowed_tokens: vec!["EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()],
        };
        let settlement = Box::new(X402Settlement::new(
            config,
            "https://seller.zeus.local/api",
            100,
        ));
        let mut mp = Marketplace::with_settlement(MarketplaceConfig::default(), settlement);
        mp.register_wallet("buyer", 500);
        mp.register_wallet("seller", 0);
        mp.list_skill(test_listing("seller", "translate", 50))
            .unwrap();

        let tx = mp
            .execute_transaction("buyer", "seller", "translate")
            .unwrap();
        assert_eq!(tx.status, TransactionStatus::Completed);
        // x402 reference should be present and mention the seller endpoint.
        let reference = tx
            .settlement_reference
            .expect("should have settlement reference");
        assert!(reference.contains("seller.zeus.local"));
        assert!(reference.contains("translate"));
    }

    // -- wallet_pay / wallet_history (web4 P0-2) ----------------------------

    #[test]
    fn test_wallet_pay_transfers_atomically() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("alice", 100);
        mp.register_wallet("bob", 10);

        mp.wallet_pay("alice", "bob", 30, Some("rent"))
            .expect("transfer should succeed");

        assert_eq!(mp.balance("alice"), Some(70));
        assert_eq!(mp.balance("bob"), Some(40));
    }

    #[test]
    fn test_wallet_pay_insufficient_funds_returns_err_no_mutation() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("alice", 20);
        mp.register_wallet("bob", 5);

        let result = mp.wallet_pay("alice", "bob", 50, None);

        // Must surface the error — not a fake success (spec line 105).
        assert!(matches!(
            result,
            Err(AgoraError::InsufficientCredits {
                available: 20,
                required: 50
            })
        ));
        // Balances must be untouched — no half-applied debit.
        assert_eq!(mp.balance("alice"), Some(20));
        assert_eq!(mp.balance("bob"), Some(5));
    }

    #[test]
    fn test_wallet_pay_unknown_wallet_errors() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("alice", 100);

        assert!(matches!(
            mp.wallet_pay("alice", "ghost", 10, None),
            Err(AgoraError::WalletNotFound(_))
        ));
        assert!(matches!(
            mp.wallet_pay("ghost", "alice", 10, None),
            Err(AgoraError::WalletNotFound(_))
        ));
        // alice untouched after both failed transfers.
        assert_eq!(mp.balance("alice"), Some(100));
    }

    #[test]
    fn test_wallet_pay_rejects_nonpositive_and_self_transfer() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("alice", 100);
        mp.register_wallet("bob", 0);

        assert!(matches!(
            mp.wallet_pay("alice", "bob", 0, None),
            Err(AgoraError::InvalidAmount(0))
        ));
        assert!(matches!(
            mp.wallet_pay("alice", "bob", -5, None),
            Err(AgoraError::InvalidAmount(-5))
        ));
        assert!(matches!(
            mp.wallet_pay("alice", "alice", 10, None),
            Err(AgoraError::InvalidAmount(10))
        ));
        assert_eq!(mp.balance("alice"), Some(100));
    }

    #[test]
    fn test_wallet_history_returns_agent_transactions() {
        let mut mp = Marketplace::with_defaults();
        mp.register_wallet("buyer", 1000);
        mp.list_skill(test_listing("seller", "translate", 50))
            .unwrap();
        mp.execute_transaction("buyer", "seller", "translate")
            .unwrap();

        let history = mp.wallet_history("buyer", None);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].buyer_agent_id, "buyer");

        // Unknown agent → empty history, not an error.
        assert!(mp.wallet_history("nobody", None).is_empty());
    }
}
