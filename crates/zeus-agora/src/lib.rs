//! The Agora — agent skill marketplace for Zeus.
//!
//! Agents list skills they can perform, other agents browse and purchase
//! executions with credits, and transactions are tracked end-to-end.

pub mod coordinator;
pub mod marketplace;
pub mod protocol;

pub use coordinator::SettlementCoordinator;

pub use marketplace::{
    CanonicalSettlement, Category, CategoryIndex, Dispute, DisputeManager, DisputeStatus,
    InMemorySettlement,
    Marketplace, MarketplaceConfig, ReputationScore, ReputationTracker, SearchQuery, SearchResult,
    SettlementProvider, SettlementReceipt, TransactionFilter, TransactionLog, TransactionOutcome,
    X402Settlement,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Transaction status
// ============================================================================

/// Status of a skill transaction between two agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Refunded,
}

// ============================================================================
// Core types
// ============================================================================

/// A skill listed for sale on the Agora.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillListing {
    /// Agent offering this skill
    pub agent_id: String,
    /// Name of the skill (unique per agent)
    pub skill_name: String,
    /// Human-readable description
    pub description: String,
    /// Price in credits per invocation
    pub price_credits: i64,
    /// JSON schema for expected input
    pub input_schema: String,
    /// JSON schema for expected output
    pub output_schema: String,
    /// Rolling average response time in milliseconds
    pub avg_response_time_ms: f64,
    /// Success rate as fraction 0.0–1.0
    pub success_rate: f64,
    /// Total number of executions completed
    pub total_executions: u64,
    /// Searchable tags (any-match filter). Populated from store rows on ingestion.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Declared capabilities (any-match filter). Typed-but-empty today;
    /// no construction site populates these (parity with legacy hardcoded `[]`).
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A transaction recording a skill purchase between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTransaction {
    /// Unique transaction identifier
    pub id: Uuid,
    /// Agent purchasing the skill execution
    pub buyer_agent_id: String,
    /// Agent providing the skill execution
    pub seller_agent_id: String,
    /// Skill being purchased
    pub skill_name: String,
    /// Credits transferred from buyer to seller
    pub credits_transferred: i64,
    /// Current transaction status
    pub status: TransactionStatus,
    /// When the transaction was created
    pub created_at: DateTime<Utc>,
    /// When the transaction completed (or failed/refunded)
    pub completed_at: Option<DateTime<Utc>>,
    /// Settlement reference from the provider (e.g. x402 payment signature).
    /// None for in-memory settlements.
    #[serde(default)]
    pub settlement_reference: Option<String>,
}

/// An agent's credit wallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWallet {
    /// Agent that owns this wallet
    pub agent_id: String,
    /// Current credit balance
    pub balance: i64,
    /// Lifetime credits earned from selling skills
    pub total_earned: i64,
    /// Lifetime credits spent purchasing skills
    pub total_spent: i64,
}

// ============================================================================
// Constructors
// ============================================================================

impl SkillListing {
    /// Create a new listing with zero stats.
    pub fn new(
        agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        description: impl Into<String>,
        price_credits: i64,
        input_schema: impl Into<String>,
        output_schema: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            skill_name: skill_name.into(),
            description: description.into(),
            price_credits,
            input_schema: input_schema.into(),
            output_schema: output_schema.into(),
            avg_response_time_ms: 0.0,
            success_rate: 1.0,
            total_executions: 0,
            tags: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    /// Attach searchable tags (builder; used by store-row ingestion).
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Attach declared capabilities (builder).
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }
}

impl SkillTransaction {
    /// Create a new pending transaction.
    pub fn new(
        buyer_agent_id: impl Into<String>,
        seller_agent_id: impl Into<String>,
        skill_name: impl Into<String>,
        credits_transferred: i64,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            buyer_agent_id: buyer_agent_id.into(),
            seller_agent_id: seller_agent_id.into(),
            skill_name: skill_name.into(),
            credits_transferred,
            status: TransactionStatus::Pending,
            created_at: Utc::now(),
            completed_at: None,
            settlement_reference: None,
        }
    }

    /// Mark the transaction as completed.
    pub fn complete(&mut self) {
        self.status = TransactionStatus::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Mark the transaction as failed.
    pub fn fail(&mut self) {
        self.status = TransactionStatus::Failed;
        self.completed_at = Some(Utc::now());
    }

    /// Refund the transaction.
    pub fn refund(&mut self) {
        self.status = TransactionStatus::Refunded;
        self.completed_at = Some(Utc::now());
    }
}

impl AgentWallet {
    /// Create a wallet with an initial balance.
    pub fn new(agent_id: impl Into<String>, initial_balance: i64) -> Self {
        Self {
            agent_id: agent_id.into(),
            balance: initial_balance,
            total_earned: 0,
            total_spent: 0,
        }
    }

    /// Deduct credits for a purchase. Returns error if insufficient funds.
    pub fn spend(&mut self, amount: i64) -> Result<(), AgoraError> {
        if amount <= 0 {
            return Err(AgoraError::InvalidAmount(amount));
        }
        if self.balance < amount {
            return Err(AgoraError::InsufficientCredits {
                available: self.balance,
                required: amount,
            });
        }
        self.balance -= amount;
        self.total_spent += amount;
        Ok(())
    }

    /// Add credits earned from a sale.
    pub fn earn(&mut self, amount: i64) -> Result<(), AgoraError> {
        if amount <= 0 {
            return Err(AgoraError::InvalidAmount(amount));
        }
        self.balance += amount;
        self.total_earned += amount;
        Ok(())
    }
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum AgoraError {
    #[error("insufficient credits: have {available}, need {required}")]
    InsufficientCredits { available: i64, required: i64 },
    #[error("invalid amount: {0}")]
    InvalidAmount(i64),
    #[error("listing not found: {agent_id}/{skill_name}")]
    ListingNotFound {
        agent_id: String,
        skill_name: String,
    },
    #[error("transaction not found: {0}")]
    TransactionNotFound(Uuid),
    #[error("settlement failed: {0}")]
    SettlementFailed(String),
    #[error("wallet not found: {0}")]
    WalletNotFound(String),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_listing_new() {
        let listing = SkillListing::new(
            "agent-1",
            "summarize",
            "Summarize text",
            10,
            r#"{"type":"object","properties":{"text":{"type":"string"}}}"#,
            r#"{"type":"string"}"#,
        );
        assert_eq!(listing.agent_id, "agent-1");
        assert_eq!(listing.skill_name, "summarize");
        assert_eq!(listing.price_credits, 10);
        assert_eq!(listing.total_executions, 0);
        assert!((listing.success_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_skill_listing_serde() {
        let listing = SkillListing::new("a", "s", "d", 5, "{}", "{}");
        let json = serde_json::to_string(&listing).expect("should serialize to JSON");
        let parsed: SkillListing = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.agent_id, "a");
        assert_eq!(parsed.price_credits, 5);
    }

    #[test]
    fn test_transaction_lifecycle() {
        let mut tx = SkillTransaction::new("buyer", "seller", "code_review", 25);
        assert_eq!(tx.status, TransactionStatus::Pending);
        assert!(tx.completed_at.is_none());

        tx.complete();
        assert_eq!(tx.status, TransactionStatus::Completed);
        assert!(tx.completed_at.is_some());
    }

    #[test]
    fn test_transaction_fail() {
        let mut tx = SkillTransaction::new("b", "s", "skill", 10);
        tx.fail();
        assert_eq!(tx.status, TransactionStatus::Failed);
        assert!(tx.completed_at.is_some());
    }

    #[test]
    fn test_transaction_refund() {
        let mut tx = SkillTransaction::new("b", "s", "skill", 10);
        tx.refund();
        assert_eq!(tx.status, TransactionStatus::Refunded);
        assert!(tx.completed_at.is_some());
    }

    #[test]
    fn test_transaction_serde() {
        let tx = SkillTransaction::new("buyer", "seller", "analyze", 50);
        let json = serde_json::to_string(&tx).expect("should serialize to JSON");
        let parsed: SkillTransaction =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.buyer_agent_id, "buyer");
        assert_eq!(parsed.seller_agent_id, "seller");
        assert_eq!(parsed.credits_transferred, 50);
        assert_eq!(parsed.status, TransactionStatus::Pending);
    }

    #[test]
    fn test_transaction_status_serde() {
        let statuses = [
            (TransactionStatus::Pending, "\"pending\""),
            (TransactionStatus::InProgress, "\"in_progress\""),
            (TransactionStatus::Completed, "\"completed\""),
            (TransactionStatus::Failed, "\"failed\""),
            (TransactionStatus::Refunded, "\"refunded\""),
        ];
        for (status, expected) in statuses {
            let json = serde_json::to_string(&status).expect("should serialize to JSON");
            assert_eq!(json, expected);
            let parsed: TransactionStatus =
                serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn test_wallet_new() {
        let wallet = AgentWallet::new("agent-x", 100);
        assert_eq!(wallet.agent_id, "agent-x");
        assert_eq!(wallet.balance, 100);
        assert_eq!(wallet.total_earned, 0);
        assert_eq!(wallet.total_spent, 0);
    }

    #[test]
    fn test_wallet_spend() {
        let mut wallet = AgentWallet::new("a", 100);
        wallet.spend(30).expect("spend should succeed");
        assert_eq!(wallet.balance, 70);
        assert_eq!(wallet.total_spent, 30);
    }

    #[test]
    fn test_wallet_spend_insufficient() {
        let mut wallet = AgentWallet::new("a", 10);
        let err = wallet.spend(20).unwrap_err();
        assert!(matches!(
            err,
            AgoraError::InsufficientCredits {
                available: 10,
                required: 20
            }
        ));
        assert_eq!(wallet.balance, 10); // unchanged
    }

    #[test]
    fn test_wallet_spend_invalid_amount() {
        let mut wallet = AgentWallet::new("a", 100);
        assert!(wallet.spend(0).is_err());
        assert!(wallet.spend(-5).is_err());
    }

    #[test]
    fn test_wallet_earn() {
        let mut wallet = AgentWallet::new("a", 50);
        wallet.earn(25).expect("earn should succeed");
        assert_eq!(wallet.balance, 75);
        assert_eq!(wallet.total_earned, 25);
    }

    #[test]
    fn test_wallet_earn_invalid_amount() {
        let mut wallet = AgentWallet::new("a", 50);
        assert!(wallet.earn(0).is_err());
        assert!(wallet.earn(-1).is_err());
    }

    #[test]
    fn test_wallet_serde() {
        let wallet = AgentWallet::new("agent-z", 500);
        let json = serde_json::to_string(&wallet).expect("should serialize to JSON");
        let parsed: AgentWallet = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.agent_id, "agent-z");
        assert_eq!(parsed.balance, 500);
    }

    #[test]
    fn test_full_purchase_flow() {
        let mut buyer = AgentWallet::new("buyer", 100);
        let mut seller = AgentWallet::new("seller", 0);
        let listing = SkillListing::new("seller", "translate", "Translate text", 15, "{}", "{}");

        // Create transaction
        let mut tx = SkillTransaction::new(
            &buyer.agent_id,
            &seller.agent_id,
            &listing.skill_name,
            listing.price_credits,
        );
        assert_eq!(tx.status, TransactionStatus::Pending);

        // Transfer credits
        buyer
            .spend(listing.price_credits)
            .expect("spend should succeed");
        seller
            .earn(listing.price_credits)
            .expect("earn should succeed");
        tx.complete();

        assert_eq!(buyer.balance, 85);
        assert_eq!(seller.balance, 15);
        assert_eq!(tx.status, TransactionStatus::Completed);
    }
}
