//! Zeus Economy — SQLite-backed token/credit economy
//!
//! Provides durable, ACID-compliant agent wallet management with:
//! - Per-agent balance tracking (SQLite)
//! - Typed transactions: Earn, Spend, Transfer, Mint, Burn
//! - Atomic multi-party settlements (buyer + seller + fee in one tx)
//! - Overdraft prevention at the database level
//! - Full audit log with queryable history
//! - Agent wallet abstraction with balance + history

mod db;
pub mod domain;
use chrono::{DateTime, Utc};
pub use domain::{
    DomainMetadata, DomainRecord, DomainRegistry, DomainRegistryConfig, DomainResult,
    DomainTransfer,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info};
use uuid::Uuid;
use zeus_core::{Error, Result};

// ===========================================================================
// Amount safety
// ===========================================================================

/// Hard upper bound on any single economic amount (mint/earn/spend/transfer/
/// stake/burn/settle). Chosen well below `i64::MAX` so that:
///   1. any `u64 > i64::MAX` is rejected (SQLite stores signed 64-bit ints —
///      a raw `u64 as i64` on such a value silently wraps negative), and
///   2. no realistic accumulation of capped amounts can approach the i64
///      ceiling, keeping `total_*` counters and `total_supply` sane.
///
/// 1 trillion base units is far above any legitimate economic operation.
pub const MAX_AMOUNT: u64 = 1_000_000_000_000;

/// Reject amounts that are zero or exceed [`MAX_AMOUNT`]. Every value-moving
/// entry point runs this before touching the ledger, so overflow can never be
/// reached from a client-supplied amount.
fn validate_amount(amount: u64) -> Result<()> {
    if amount == 0 {
        return Err(Error::Validation("Amount must be > 0".to_string()));
    }
    if amount > MAX_AMOUNT {
        return Err(Error::Validation(format!(
            "Amount {amount} exceeds maximum allowed {MAX_AMOUNT}"
        )));
    }
    Ok(())
}

// ===========================================================================
// Transaction types
// ===========================================================================

/// Category of economic activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionKind {
    /// Agent earned tokens (task completion, marketplace sale, review reward).
    Earn,
    /// Agent spent tokens (LLM call, tool use, agent birth, idle cost).
    Spend,
    /// Peer-to-peer transfer (marketplace trade).
    Transfer,
    /// System mints new tokens (human task injection).
    Mint,
    /// Tokens destroyed (LLM API cost settlement).
    Burn,
    /// Platform fee collected from a trade.
    Fee,
    /// Unrecognised kind read from DB — preserves the raw value for audit.
    Unknown(String),
}

impl TransactionKind {
    fn as_str(&self) -> String {
        match self {
            Self::Earn => "earn".to_string(),
            Self::Spend => "spend".to_string(),
            Self::Transfer => "transfer".to_string(),
            Self::Mint => "mint".to_string(),
            Self::Burn => "burn".to_string(),
            Self::Fee => "fee".to_string(),
            Self::Unknown(s) => format!("unknown:{s}"),
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "earn" => Self::Earn,
            "spend" => Self::Spend,
            "transfer" => Self::Transfer,
            "mint" => Self::Mint,
            "burn" => Self::Burn,
            "fee" => Self::Fee,
            other => {
                tracing::warn!(kind = other, "Unknown TransactionKind read from DB");
                Self::Unknown(other.to_string())
            }
        }
    }
}

/// Reason sub-category for richer audit trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionReason {
    // Earn reasons
    TaskCompletion,
    MarketplaceSale,
    ReviewReward,
    // Spend reasons
    LlmCall,
    ToolUse,
    AgentSpawn,
    IdleCost,
    // Transfer reasons
    MarketplaceTrade,
    PeerTransfer,
    // Mint/Burn
    HumanTaskInjection,
    ApiCostSettlement,
    SystemGrant,
    // Fee
    PlatformFee,
    // Generic
    Custom(String),
}

impl TransactionReason {
    fn as_str(&self) -> String {
        match self {
            Self::TaskCompletion => "task_completion".to_string(),
            Self::MarketplaceSale => "marketplace_sale".to_string(),
            Self::ReviewReward => "review_reward".to_string(),
            Self::LlmCall => "llm_call".to_string(),
            Self::ToolUse => "tool_use".to_string(),
            Self::AgentSpawn => "agent_spawn".to_string(),
            Self::IdleCost => "idle_cost".to_string(),
            Self::MarketplaceTrade => "marketplace_trade".to_string(),
            Self::PeerTransfer => "peer_transfer".to_string(),
            Self::HumanTaskInjection => "human_task_injection".to_string(),
            Self::ApiCostSettlement => "api_cost_settlement".to_string(),
            Self::SystemGrant => "system_grant".to_string(),
            Self::PlatformFee => "platform_fee".to_string(),
            Self::Custom(s) => format!("custom:{s}"),
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "task_completion" => Self::TaskCompletion,
            "marketplace_sale" => Self::MarketplaceSale,
            "review_reward" => Self::ReviewReward,
            "llm_call" => Self::LlmCall,
            "tool_use" => Self::ToolUse,
            "agent_spawn" => Self::AgentSpawn,
            "idle_cost" => Self::IdleCost,
            "marketplace_trade" => Self::MarketplaceTrade,
            "peer_transfer" => Self::PeerTransfer,
            "human_task_injection" => Self::HumanTaskInjection,
            "api_cost_settlement" => Self::ApiCostSettlement,
            "system_grant" => Self::SystemGrant,
            "platform_fee" => Self::PlatformFee,
            other => {
                if let Some(custom) = other.strip_prefix("custom:") {
                    Self::Custom(custom.to_string())
                } else {
                    Self::Custom(other.to_string())
                }
            }
        }
    }
}

/// A recorded transaction in the audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub kind: TransactionKind,
    pub reason: TransactionReason,
    /// Source agent (None for mints).
    pub from_agent: Option<String>,
    /// Destination agent (None for burns).
    pub to_agent: Option<String>,
    pub amount: u64,
    /// Optional reference to a related entity (trade ID, session ID, etc).
    pub reference_id: Option<String>,
    /// Free-form note.
    pub note: String,
    pub created_at: DateTime<Utc>,
}

// ===========================================================================
// Agent Wallet (read-only view)
// ===========================================================================

/// Snapshot of an agent's economic state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWallet {
    pub agent_id: String,
    pub balance: u64,
    pub total_earned: u64,
    pub total_spent: u64,
    pub total_transferred_in: u64,
    pub total_transferred_out: u64,
    pub transaction_count: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ===========================================================================
// TokenLedger — SQLite-backed
// ===========================================================================

/// SQLite-backed token ledger providing ACID balance management.
///
/// Each method opens a fresh connection (consistent with Zeus crate patterns).
/// Atomic multi-party transactions use SQLite's IMMEDIATE transaction mode
/// to prevent TOCTOU races on balance checks.
/// (stake_id, agent_id, amount, purpose, status, created_at, released_at)
pub type StakeRow = (String, String, u64, String, String, String, Option<String>);

pub struct TokenLedger {
    path: PathBuf,
}

const ECONOMY_MIGRATIONS: &[&str] = &[
    // v1: initial schema
    "CREATE TABLE IF NOT EXISTS wallets (
                agent_id TEXT PRIMARY KEY,
                balance INTEGER NOT NULL DEFAULT 0,
                total_earned INTEGER NOT NULL DEFAULT 0,
                total_spent INTEGER NOT NULL DEFAULT 0,
                total_transferred_in INTEGER NOT NULL DEFAULT 0,
                total_transferred_out INTEGER NOT NULL DEFAULT 0,
                transaction_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS transactions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                reason TEXT NOT NULL,
                from_agent TEXT,
                to_agent TEXT,
                amount INTEGER NOT NULL,
                reference_id TEXT,
                note TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_tx_from ON transactions(from_agent);
            CREATE INDEX IF NOT EXISTS idx_tx_to ON transactions(to_agent);
            CREATE INDEX IF NOT EXISTS idx_tx_kind ON transactions(kind);
            CREATE INDEX IF NOT EXISTS idx_tx_created ON transactions(created_at);
            CREATE INDEX IF NOT EXISTS idx_tx_reference ON transactions(reference_id);",
    // v2: add token column for multi-token balance isolation
    "ALTER TABLE wallets ADD COLUMN token TEXT NOT NULL DEFAULT 'ZEUS';
     ALTER TABLE transactions ADD COLUMN token TEXT NOT NULL DEFAULT 'ZEUS';
     CREATE INDEX IF NOT EXISTS idx_tx_token ON transactions(token);",
    // v3: stakes ledger — records real collateral holds so unstake cannot
    // counterfeit tokens. A stake is created by `stake()` (debits wallet) and
    // consumed by `unstake()` (credits wallet), validated against this table.
    // status: 'active' | 'released'. amount is the ORIGINAL staked amount;
    // unstake must release the full remaining amount (no partial-unstake mint).
    "CREATE TABLE IF NOT EXISTS stakes (
                stake_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                amount INTEGER NOT NULL,
                purpose TEXT NOT NULL DEFAULT 'general',
                status TEXT NOT NULL DEFAULT 'active',
                created_at TEXT NOT NULL,
                released_at TEXT
            );
     CREATE INDEX IF NOT EXISTS idx_stakes_agent ON stakes(agent_id);
     CREATE INDEX IF NOT EXISTS idx_stakes_status ON stakes(status);",
    // v4: add a DB-level CHECK(balance >= 0) backstop. SQLite cannot attach a
    // CHECK to an existing column via ALTER, so we rebuild the wallets table
    // and copy rows across. This is defense-in-depth: the application already
    // guards overdraft inside each IMMEDIATE tx, but a buggy/direct write path
    // now hits a hard constraint instead of silently corrupting the ledger.
    "CREATE TABLE wallets_v4 (
                agent_id TEXT PRIMARY KEY,
                balance INTEGER NOT NULL DEFAULT 0 CHECK (balance >= 0),
                total_earned INTEGER NOT NULL DEFAULT 0,
                total_spent INTEGER NOT NULL DEFAULT 0,
                total_transferred_in INTEGER NOT NULL DEFAULT 0,
                total_transferred_out INTEGER NOT NULL DEFAULT 0,
                transaction_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                token TEXT NOT NULL DEFAULT 'ZEUS'
            );
     INSERT INTO wallets_v4 (agent_id, balance, total_earned, total_spent,
                total_transferred_in, total_transferred_out, transaction_count,
                created_at, updated_at, token)
            SELECT agent_id, balance, total_earned, total_spent,
                total_transferred_in, total_transferred_out, transaction_count,
                created_at, updated_at, token FROM wallets;
     DROP TABLE wallets;
     ALTER TABLE wallets_v4 RENAME TO wallets;",
];

impl TokenLedger {
    /// Create a new ledger, initializing the schema.
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self> {
        let path = db_path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Database(format!("Failed to create db dir: {e}")))?;
        }
        let ledger = Self { path };
        ledger.init()?;
        Ok(ledger)
    }

    fn init(&self) -> Result<()> {
        let conn = self.conn()?;
        crate::db::run_migrations(&conn, ECONOMY_MIGRATIONS)?;
        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.path)
            .map_err(|e| Error::Database(format!("Failed to open economy db: {e}")))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| Error::Database(format!("Failed to set pragmas: {e}")))?;
        Ok(conn)
    }

    /// Ensure a wallet exists for an agent, creating with zero balance if absent.
    fn ensure_wallet(conn: &Connection, agent_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO wallets (agent_id, balance, total_earned, total_spent,
             total_transferred_in, total_transferred_out, transaction_count, created_at, updated_at)
             VALUES (?1, 0, 0, 0, 0, 0, 0, ?2, ?2)",
            params![agent_id, now],
        )
        .map_err(|e| Error::Database(format!("Failed to ensure wallet: {e}")))?;
        Ok(())
    }

    fn record_tx(conn: &Connection, tx: &Transaction) -> Result<()> {
        conn.execute(
            "INSERT INTO transactions (id, kind, reason, from_agent, to_agent, amount, reference_id, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                tx.id,
                tx.kind.as_str(),
                tx.reason.as_str(),
                tx.from_agent,
                tx.to_agent,
                tx.amount as i64,
                tx.reference_id,
                tx.note,
                tx.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| Error::Database(format!("Failed to record transaction: {e}")))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Earn
    // -----------------------------------------------------------------------

    /// Credit tokens to an agent for completing work.
    pub fn earn(
        &self,
        agent_id: &str,
        amount: u64,
        reason: TransactionReason,
        note: impl Into<String>,
    ) -> Result<u64> {
        validate_amount(amount)?;
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, agent_id)?;

        let current = Self::read_balance(&db_tx, agent_id)?;
        current
            .checked_add(amount)
            .ok_or_else(|| Error::Validation("Earn would overflow balance".to_string()))?;

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance + ?1, total_earned = total_earned + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, Utc::now().to_rfc3339(), agent_id],
            )
            .map_err(|e| Error::Database(format!("Earn update failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Earn,
            reason,
            from_agent: None,
            to_agent: Some(agent_id.to_string()),
            amount,
            reference_id: None,
            note: note.into(),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        let balance = Self::read_balance(&db_tx, agent_id)?;
        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        debug!(agent_id, amount, balance, "Tokens earned");
        Ok(balance)
    }

    // -----------------------------------------------------------------------
    // Spend
    // -----------------------------------------------------------------------

    /// Debit tokens from an agent for resource consumption.
    /// Returns error if balance is insufficient.
    pub fn spend(
        &self,
        agent_id: &str,
        amount: u64,
        reason: TransactionReason,
        note: impl Into<String>,
    ) -> Result<u64> {
        validate_amount(amount)?;
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, agent_id)?;

        let current = Self::read_balance(&db_tx, agent_id)?;
        if current < amount {
            return Err(Error::Validation(format!(
                "Insufficient balance: agent {agent_id} has {current}, needs {amount}"
            )));
        }

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance - ?1, total_spent = total_spent + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, Utc::now().to_rfc3339(), agent_id],
            )
            .map_err(|e| Error::Database(format!("Spend update failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Spend,
            reason,
            from_agent: Some(agent_id.to_string()),
            to_agent: None,
            amount,
            reference_id: None,
            note: note.into(),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        let balance = Self::read_balance(&db_tx, agent_id)?;
        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        debug!(agent_id, amount, balance, "Tokens spent");
        Ok(balance)
    }

    // -----------------------------------------------------------------------
    // Transfer
    // -----------------------------------------------------------------------

    /// Transfer tokens between two agents. Overdraft-safe.
    pub fn transfer(
        &self,
        from: &str,
        to: &str,
        amount: u64,
        reason: TransactionReason,
        note: impl Into<String>,
    ) -> Result<(u64, u64)> {
        validate_amount(amount)?;
        let note_str: String = note.into();
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, from)?;
        Self::ensure_wallet(&db_tx, to)?;

        let from_balance = Self::read_balance(&db_tx, from)?;
        if from_balance < amount {
            return Err(Error::Validation(format!(
                "Insufficient balance: agent {from} has {from_balance}, needs {amount}"
            )));
        }

        // Guard the credit side against overflow before mutating either wallet.
        let to_balance = Self::read_balance(&db_tx, to)?;
        to_balance
            .checked_add(amount)
            .ok_or_else(|| Error::Validation("Transfer would overflow recipient balance".to_string()))?;

        let now = Utc::now().to_rfc3339();

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance - ?1, total_transferred_out = total_transferred_out + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, now, from],
            )
            .map_err(|e| Error::Database(format!("Transfer debit failed: {e}")))?;

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance + ?1, total_transferred_in = total_transferred_in + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, now, to],
            )
            .map_err(|e| Error::Database(format!("Transfer credit failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Transfer,
            reason,
            from_agent: Some(from.to_string()),
            to_agent: Some(to.to_string()),
            amount,
            reference_id: None,
            note: note_str,
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        let new_from = Self::read_balance(&db_tx, from)?;
        let new_to = Self::read_balance(&db_tx, to)?;

        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(from, to, amount, "Token transfer");
        Ok((new_from, new_to))
    }

    // -----------------------------------------------------------------------
    // Atomic multi-party settlement
    // -----------------------------------------------------------------------

    /// Execute an atomic marketplace trade: buyer pays, seller receives, platform takes fee.
    /// All three mutations happen in a single SQLite transaction.
    /// Returns (buyer_balance, seller_balance, fee_collector_balance).
    pub fn settle_trade(
        &self,
        buyer: &str,
        seller: &str,
        fee_collector: &str,
        total_price: u64,
        fee_amount: u64,
        reference_id: impl Into<String>,
    ) -> Result<(u64, u64, u64)> {
        validate_amount(total_price)?;
        if fee_amount > total_price {
            return Err(Error::Validation(
                "Fee cannot exceed total price".to_string(),
            ));
        }
        let seller_amount = total_price - fee_amount;
        let ref_id: String = reference_id.into();

        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, buyer)?;
        Self::ensure_wallet(&db_tx, seller)?;
        Self::ensure_wallet(&db_tx, fee_collector)?;

        let buyer_balance = Self::read_balance(&db_tx, buyer)?;
        if buyer_balance < total_price {
            return Err(Error::Validation(format!(
                "Insufficient balance: buyer {buyer} has {buyer_balance}, needs {total_price}"
            )));
        }

        // Guard both credit sides against overflow before any mutation.
        Self::read_balance(&db_tx, seller)?
            .checked_add(seller_amount)
            .ok_or_else(|| Error::Validation("Settle would overflow seller balance".to_string()))?;
        Self::read_balance(&db_tx, fee_collector)?
            .checked_add(fee_amount)
            .ok_or_else(|| {
                Error::Validation("Settle would overflow fee-collector balance".to_string())
            })?;

        let now = Utc::now().to_rfc3339();

        // Debit buyer
        db_tx
            .execute(
                "UPDATE wallets SET balance = balance - ?1, total_spent = total_spent + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![total_price as i64, now, buyer],
            )
            .map_err(|e| Error::Database(format!("Buyer debit failed: {e}")))?;

        // Credit seller
        db_tx
            .execute(
                "UPDATE wallets SET balance = balance + ?1, total_earned = total_earned + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![seller_amount as i64, now, seller],
            )
            .map_err(|e| Error::Database(format!("Seller credit failed: {e}")))?;

        // Credit fee collector
        if fee_amount > 0 {
            db_tx
                .execute(
                    "UPDATE wallets SET balance = balance + ?1, total_earned = total_earned + ?1,
                     transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                    params![fee_amount as i64, now, fee_collector],
                )
                .map_err(|e| Error::Database(format!("Fee credit failed: {e}")))?;
        }

        // Record trade transaction (buyer -> seller)
        let trade_tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Transfer,
            reason: TransactionReason::MarketplaceTrade,
            from_agent: Some(buyer.to_string()),
            to_agent: Some(seller.to_string()),
            amount: seller_amount,
            reference_id: Some(ref_id.clone()),
            note: format!("Trade settlement: {total_price} total, {fee_amount} fee"),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &trade_tx)?;

        // Record fee transaction if non-zero
        if fee_amount > 0 {
            let fee_tx = Transaction {
                id: Uuid::new_v4().to_string(),
                kind: TransactionKind::Fee,
                reason: TransactionReason::PlatformFee,
                from_agent: Some(buyer.to_string()),
                to_agent: Some(fee_collector.to_string()),
                amount: fee_amount,
                reference_id: Some(ref_id),
                note: "Platform fee".to_string(),
                created_at: Utc::now(),
            };
            Self::record_tx(&db_tx, &fee_tx)?;
        }

        let new_buyer = Self::read_balance(&db_tx, buyer)?;
        let new_seller = Self::read_balance(&db_tx, seller)?;
        let new_fee = Self::read_balance(&db_tx, fee_collector)?;

        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(
            buyer,
            seller, fee_collector, total_price, fee_amount, "Trade settled"
        );
        Ok((new_buyer, new_seller, new_fee))
    }

    // -----------------------------------------------------------------------
    // Stake & Unstake
    // -----------------------------------------------------------------------

    /// Stake tokens as collateral. Atomically debits the agent's wallet and
    /// records an `active` stake row in a single IMMEDIATE transaction, so the
    /// debit and the stake record can never diverge. Returns the new
    /// `stake_id` and the agent's post-debit balance.
    ///
    /// Overdraft-safe: the balance check is inside the transaction.
    pub fn stake(
        &self,
        agent_id: &str,
        amount: u64,
        purpose: impl Into<String>,
    ) -> Result<(String, u64)> {
        validate_amount(amount)?;
        let purpose = purpose.into();
        let stake_id = Uuid::new_v4().to_string();

        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, agent_id)?;

        let current = Self::read_balance(&db_tx, agent_id)?;
        if current < amount {
            return Err(Error::Validation(format!(
                "Insufficient balance to stake: agent {agent_id} has {current}, needs {amount}"
            )));
        }

        let new_balance = current
            .checked_sub(amount)
            .ok_or_else(|| Error::Validation("Stake would underflow balance".to_string()))?;

        db_tx
            .execute(
                "UPDATE wallets SET balance = ?1, total_spent = total_spent + ?2,
                 transaction_count = transaction_count + 1, updated_at = ?3 WHERE agent_id = ?4",
                params![
                    new_balance as i64,
                    amount as i64,
                    Utc::now().to_rfc3339(),
                    agent_id
                ],
            )
            .map_err(|e| Error::Database(format!("Stake debit failed: {e}")))?;

        db_tx
            .execute(
                "INSERT INTO stakes (stake_id, agent_id, amount, purpose, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
                params![
                    stake_id,
                    agent_id,
                    amount as i64,
                    purpose,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| Error::Database(format!("Stake record failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Spend,
            reason: TransactionReason::Custom(format!("stake:{purpose}")),
            from_agent: Some(agent_id.to_string()),
            to_agent: None,
            amount,
            reference_id: Some(stake_id.clone()),
            note: format!("Staked {amount} tokens for {purpose}"),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(agent_id, amount, stake_id, purpose, "Tokens staked");
        Ok((stake_id, new_balance))
    }

    /// Release a previously recorded stake, crediting the ORIGINAL staked
    /// amount back to the owning agent. Validates that the stake exists, is
    /// still `active`, and belongs to `agent_id` — then marks it `released` in
    /// the same transaction. This is what makes unstake non-counterfeitable:
    /// you can only ever get back exactly what a real `stake()` locked, once.
    ///
    /// Returns the released amount and the agent's post-credit balance.
    pub fn unstake(&self, agent_id: &str, stake_id: &str) -> Result<(u64, u64)> {
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        // Look up the stake and validate ownership + status atomically.
        let row: Option<(String, i64, String)> = db_tx
            .query_row(
                "SELECT agent_id, amount, status FROM stakes WHERE stake_id = ?1",
                params![stake_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| Error::Database(format!("Stake lookup failed: {e}")))?;

        let (owner, amount_i64, status) = row.ok_or_else(|| {
            Error::Validation(format!("No such stake: {stake_id}"))
        })?;

        if owner != agent_id {
            return Err(Error::Validation(format!(
                "Stake {stake_id} does not belong to agent {agent_id}"
            )));
        }
        if status != "active" {
            return Err(Error::Validation(format!(
                "Stake {stake_id} is not active (status: {status})"
            )));
        }

        let amount = amount_i64 as u64;

        // Mark released FIRST (idempotency guard against concurrent unstake of
        // the same stake — the IMMEDIATE tx serializes, and the status check
        // above already read 'active' under this lock).
        let updated = db_tx
            .execute(
                "UPDATE stakes SET status = 'released', released_at = ?1
                 WHERE stake_id = ?2 AND status = 'active'",
                params![Utc::now().to_rfc3339(), stake_id],
            )
            .map_err(|e| Error::Database(format!("Stake release failed: {e}")))?;
        if updated != 1 {
            return Err(Error::Validation(format!(
                "Stake {stake_id} could not be released (already consumed)"
            )));
        }

        Self::ensure_wallet(&db_tx, agent_id)?;
        let current = Self::read_balance(&db_tx, agent_id)?;
        let new_balance = current
            .checked_add(amount)
            .ok_or_else(|| Error::Validation("Unstake would overflow balance".to_string()))?;

        db_tx
            .execute(
                "UPDATE wallets SET balance = ?1, total_earned = total_earned + ?2,
                 transaction_count = transaction_count + 1, updated_at = ?3 WHERE agent_id = ?4",
                params![
                    new_balance as i64,
                    amount as i64,
                    Utc::now().to_rfc3339(),
                    agent_id
                ],
            )
            .map_err(|e| Error::Database(format!("Unstake credit failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Earn,
            reason: TransactionReason::Custom(format!("unstake:{stake_id}")),
            from_agent: None,
            to_agent: Some(agent_id.to_string()),
            amount,
            reference_id: Some(stake_id.to_string()),
            note: format!("Unstaked {amount} tokens (stake:{stake_id})"),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(agent_id, amount, stake_id, "Tokens unstaked");
        Ok((amount, new_balance))
    }

    /// List stakes, optionally filtered by agent_id and/or status.
    /// Returns (stake_id, agent_id, amount, purpose, status, created_at, released_at).
    pub fn list_stakes(
        &self,
        agent_id: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<StakeRow>> {
        let conn = self.conn()?;
        let mut sql = String::from(
            "SELECT stake_id, agent_id, amount, purpose, status, created_at, released_at FROM stakes WHERE 1=1"
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1;

        if let Some(aid) = agent_id {
            sql.push_str(&format!(" AND agent_id = ?{idx}"));
            param_values.push(Box::new(aid.to_string()));
            idx += 1;
        }
        if let Some(st) = status {
            sql.push_str(&format!(" AND status = ?{idx}"));
            param_values.push(Box::new(st.to_string()));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let mut stmt = conn.prepare(&sql).map_err(|e| Error::Database(format!("Prepare list_stakes failed: {e}")))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|e| Error::Database(format!("Query list_stakes failed: {e}")))?;

        let mut results = Vec::new();
        for row in rows {
            let r = row.map_err(|e| Error::Database(format!("Row read failed: {e}")))?;
            results.push(r);
        }
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Mint & Burn
    // -----------------------------------------------------------------------

    /// System mints new tokens into an agent's wallet (e.g., human task injection).
    pub fn mint(
        &self,
        agent_id: &str,
        amount: u64,
        reason: TransactionReason,
        note: impl Into<String>,
    ) -> Result<u64> {
        validate_amount(amount)?;
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, agent_id)?;

        let current = Self::read_balance(&db_tx, agent_id)?;
        current
            .checked_add(amount)
            .ok_or_else(|| Error::Validation("Mint would overflow balance".to_string()))?;

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance + ?1, total_earned = total_earned + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, Utc::now().to_rfc3339(), agent_id],
            )
            .map_err(|e| Error::Database(format!("Mint update failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Mint,
            reason,
            from_agent: None,
            to_agent: Some(agent_id.to_string()),
            amount,
            reference_id: None,
            note: note.into(),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        let balance = Self::read_balance(&db_tx, agent_id)?;
        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(agent_id, amount, balance, "Tokens minted");
        Ok(balance)
    }

    /// Burn tokens from an agent's wallet (e.g., settling LLM API costs).
    /// Returns error if balance is insufficient.
    pub fn burn(
        &self,
        agent_id: &str,
        amount: u64,
        reason: TransactionReason,
        note: impl Into<String>,
    ) -> Result<u64> {
        validate_amount(amount)?;
        let mut conn = self.conn()?;
        let db_tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| Error::Database(format!("Begin tx failed: {e}")))?;

        Self::ensure_wallet(&db_tx, agent_id)?;

        let current = Self::read_balance(&db_tx, agent_id)?;
        if current < amount {
            return Err(Error::Validation(format!(
                "Insufficient balance for burn: agent {agent_id} has {current}, needs {amount}"
            )));
        }

        db_tx
            .execute(
                "UPDATE wallets SET balance = balance - ?1, total_spent = total_spent + ?1,
                 transaction_count = transaction_count + 1, updated_at = ?2 WHERE agent_id = ?3",
                params![amount as i64, Utc::now().to_rfc3339(), agent_id],
            )
            .map_err(|e| Error::Database(format!("Burn update failed: {e}")))?;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            kind: TransactionKind::Burn,
            reason,
            from_agent: Some(agent_id.to_string()),
            to_agent: None,
            amount,
            reference_id: None,
            note: note.into(),
            created_at: Utc::now(),
        };
        Self::record_tx(&db_tx, &tx)?;

        let balance = Self::read_balance(&db_tx, agent_id)?;
        db_tx
            .commit()
            .map_err(|e| Error::Database(format!("Commit failed: {e}")))?;

        info!(agent_id, amount, balance, "Tokens burned");
        Ok(balance)
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Get current balance for an agent.
    pub fn balance(&self, agent_id: &str) -> Result<u64> {
        let conn = self.conn()?;
        Self::ensure_wallet(&conn, agent_id)?;
        Self::read_balance(&conn, agent_id)
    }

    /// Get the full wallet snapshot for an agent.
    pub fn wallet(&self, agent_id: &str) -> Result<AgentWallet> {
        let conn = self.conn()?;
        Self::ensure_wallet(&conn, agent_id)?;

        let mut stmt = conn
            .prepare(
                "SELECT agent_id, balance, total_earned, total_spent,
                 total_transferred_in, total_transferred_out, transaction_count,
                 created_at, updated_at FROM wallets WHERE agent_id = ?1",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        stmt.query_row(params![agent_id], |row| {
            Ok(AgentWallet {
                agent_id: row.get(0)?,
                balance: row.get::<_, i64>(1)? as u64,
                total_earned: row.get::<_, i64>(2)? as u64,
                total_spent: row.get::<_, i64>(3)? as u64,
                total_transferred_in: row.get::<_, i64>(4)? as u64,
                total_transferred_out: row.get::<_, i64>(5)? as u64,
                transaction_count: row.get::<_, i64>(6)? as u64,
                created_at: row.get::<_, String>(7)?.parse().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        7,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                updated_at: row.get::<_, String>(8)?.parse().map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
            })
        })
        .map_err(|e| Error::Database(format!("Wallet query failed: {e}")))
    }

    /// List all wallets with non-zero balance.
    pub fn all_wallets(&self) -> Result<Vec<AgentWallet>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT agent_id, balance, total_earned, total_spent,
                 total_transferred_in, total_transferred_out, transaction_count,
                 created_at, updated_at FROM wallets ORDER BY balance DESC",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        let wallets = stmt
            .query_map([], |row| {
                Ok(AgentWallet {
                    agent_id: row.get(0)?,
                    balance: row.get::<_, i64>(1)? as u64,
                    total_earned: row.get::<_, i64>(2)? as u64,
                    total_spent: row.get::<_, i64>(3)? as u64,
                    total_transferred_in: row.get::<_, i64>(4)? as u64,
                    total_transferred_out: row.get::<_, i64>(5)? as u64,
                    transaction_count: row.get::<_, i64>(6)? as u64,
                    created_at: row.get::<_, String>(7)?.parse().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                    updated_at: row.get::<_, String>(8)?.parse().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            8,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?,
                })
            })
            .map_err(|e| Error::Database(format!("Query failed: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Collect failed: {e}")))?;

        Ok(wallets)
    }

    /// Get total token supply across all wallets.
    pub fn total_supply(&self) -> Result<u64> {
        let conn = self.conn()?;
        let supply: i64 = conn
            .query_row("SELECT COALESCE(SUM(balance), 0) FROM wallets", [], |row| {
                row.get(0)
            })
            .map_err(|e| Error::Database(format!("Sum query failed: {e}")))?;
        Ok(supply as u64)
    }

    // -----------------------------------------------------------------------
    // Audit log queries
    // -----------------------------------------------------------------------

    /// Get transaction history for an agent (most recent first).
    pub fn transactions_for(&self, agent_id: &str, limit: usize) -> Result<Vec<Transaction>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, reason, from_agent, to_agent, amount, reference_id, note, created_at
                 FROM transactions
                 WHERE from_agent = ?1 OR to_agent = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        let txs = stmt
            .query_map(params![agent_id, limit as i64], Self::row_to_tx)
            .map_err(|e| Error::Database(format!("Query failed: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Collect failed: {e}")))?;

        Ok(txs)
    }

    /// Get all transactions (most recent first).
    pub fn all_transactions(&self, limit: usize) -> Result<Vec<Transaction>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, reason, from_agent, to_agent, amount, reference_id, note, created_at
                 FROM transactions ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        let txs = stmt
            .query_map(params![limit as i64], Self::row_to_tx)
            .map_err(|e| Error::Database(format!("Query failed: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Collect failed: {e}")))?;

        Ok(txs)
    }

    /// Get transactions by kind (e.g., all mints).
    pub fn transactions_by_kind(
        &self,
        kind: TransactionKind,
        limit: usize,
    ) -> Result<Vec<Transaction>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, reason, from_agent, to_agent, amount, reference_id, note, created_at
                 FROM transactions WHERE kind = ?1 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        let txs = stmt
            .query_map(params![kind.as_str(), limit as i64], Self::row_to_tx)
            .map_err(|e| Error::Database(format!("Query failed: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Collect failed: {e}")))?;

        Ok(txs)
    }

    /// Get transactions by reference ID (e.g., all entries for a trade).
    pub fn transactions_by_reference(&self, reference_id: &str) -> Result<Vec<Transaction>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, reason, from_agent, to_agent, amount, reference_id, note, created_at
                 FROM transactions WHERE reference_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| Error::Database(format!("Prepare failed: {e}")))?;

        let txs = stmt
            .query_map(params![reference_id], Self::row_to_tx)
            .map_err(|e| Error::Database(format!("Query failed: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("Collect failed: {e}")))?;

        Ok(txs)
    }

    /// Summary: total minted and total burned.
    pub fn mint_burn_summary(&self) -> Result<(u64, u64)> {
        let conn = self.conn()?;
        let minted: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE kind = 'mint'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("Mint sum failed: {e}")))?;
        let burned: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE kind = 'burn'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("Burn sum failed: {e}")))?;
        Ok((minted as u64, burned as u64))
    }

    /// Count of transactions in the ledger.
    pub fn transaction_count(&self) -> Result<u64> {
        let conn = self.conn()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transactions", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("Count failed: {e}")))?;
        Ok(count as u64)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn read_balance(conn: &Connection, agent_id: &str) -> Result<u64> {
        let balance: i64 = conn
            .query_row(
                "SELECT balance FROM wallets WHERE agent_id = ?1",
                params![agent_id],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("Balance query failed: {e}")))?;
        Ok(balance as u64)
    }

    fn row_to_tx(row: &rusqlite::Row) -> rusqlite::Result<Transaction> {
        Ok(Transaction {
            id: row.get(0)?,
            kind: TransactionKind::from_str(&row.get::<_, String>(1)?),
            reason: TransactionReason::from_str(&row.get::<_, String>(2)?),
            from_agent: row.get(3)?,
            to_agent: row.get(4)?,
            amount: row.get::<_, i64>(5)? as u64,
            reference_id: row.get(6)?,
            note: row.get(7)?,
            created_at: row.get::<_, String>(8)?.parse().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_ledger() -> (TempDir, TokenLedger) {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let ledger = TokenLedger::new(tmp.path().join("economy.db"))
            .expect("TokenLedger::new should succeed");
        (tmp, ledger)
    }

    // -- Earn tests --

    #[test]
    fn test_earn_creates_wallet_and_credits() {
        let (_tmp, ledger) = temp_ledger();
        let balance = ledger
            .earn(
                "agent-1",
                100,
                TransactionReason::TaskCompletion,
                "finished task",
            )
            .expect("earn should succeed");
        assert_eq!(balance, 100);
        assert_eq!(
            ledger.balance("agent-1").expect("balance should succeed"),
            100
        );
    }

    #[test]
    fn test_earn_accumulates() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 50, TransactionReason::TaskCompletion, "task 1")
            .expect("earn should succeed");
        let balance = ledger
            .earn("agent-1", 30, TransactionReason::ReviewReward, "review")
            .expect("earn should succeed");
        assert_eq!(balance, 80);
    }

    #[test]
    fn test_earn_marketplace_sale() {
        let (_tmp, ledger) = temp_ledger();
        let balance = ledger
            .earn(
                "seller-1",
                200,
                TransactionReason::MarketplaceSale,
                "sold skill",
            )
            .expect("earn should succeed");
        assert_eq!(balance, 200);

        let txs = ledger
            .transactions_for("seller-1", 10)
            .expect("transactions_for should succeed");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].kind, TransactionKind::Earn);
        assert_eq!(txs[0].reason, TransactionReason::MarketplaceSale);
    }

    // -- Spend tests --

    #[test]
    fn test_spend_deducts() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let balance = ledger
            .spend("agent-1", 40, TransactionReason::LlmCall, "gpt-4o call")
            .expect("spend should succeed");
        assert_eq!(balance, 60);
    }

    #[test]
    fn test_spend_tool_use() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let balance = ledger
            .spend("agent-1", 5, TransactionReason::ToolUse, "shell exec")
            .expect("spend should succeed");
        assert_eq!(balance, 95);
    }

    #[test]
    fn test_spend_agent_spawn() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let balance = ledger
            .spend(
                "agent-1",
                50,
                TransactionReason::AgentSpawn,
                "spawned subagent",
            )
            .expect("spend should succeed");
        assert_eq!(balance, 50);
    }

    #[test]
    fn test_spend_idle_cost() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let balance = ledger
            .spend("agent-1", 1, TransactionReason::IdleCost, "heartbeat tick")
            .expect("spend should succeed");
        assert_eq!(balance, 99);
    }

    // -- Overdraft prevention --

    #[test]
    fn test_spend_overdraft_prevented() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 50, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let result = ledger.spend("agent-1", 100, TransactionReason::LlmCall, "too expensive");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Insufficient balance"));
        // Balance unchanged
        assert_eq!(
            ledger.balance("agent-1").expect("balance should succeed"),
            50
        );
    }

    #[test]
    fn test_spend_exact_balance_ok() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("agent-1", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let balance = ledger
            .spend("agent-1", 100, TransactionReason::LlmCall, "exact")
            .expect("spend should succeed");
        assert_eq!(balance, 0);
    }

    #[test]
    fn test_spend_zero_balance_fails() {
        let (_tmp, ledger) = temp_ledger();
        let result = ledger.spend("agent-new", 1, TransactionReason::LlmCall, "no funds");
        assert!(result.is_err());
    }

    // -- Transfer tests --

    #[test]
    fn test_transfer_between_agents() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("alice", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let (alice_bal, bob_bal) = ledger
            .transfer(
                "alice",
                "bob",
                30,
                TransactionReason::PeerTransfer,
                "payment",
            )
            .expect("operation should succeed");
        assert_eq!(alice_bal, 70);
        assert_eq!(bob_bal, 30);
    }

    #[test]
    fn test_transfer_overdraft_prevented() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("alice", 50, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let result = ledger.transfer(
            "alice",
            "bob",
            100,
            TransactionReason::PeerTransfer,
            "too much",
        );
        assert!(result.is_err());
        assert_eq!(ledger.balance("alice").expect("balance should succeed"), 50);
        assert_eq!(ledger.balance("bob").expect("balance should succeed"), 0);
    }

    #[test]
    fn test_transfer_creates_receiver_wallet() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("alice", 100, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        let (_, bob_bal) = ledger
            .transfer(
                "alice",
                "bob",
                25,
                TransactionReason::MarketplaceTrade,
                "trade",
            )
            .expect("operation should succeed");
        assert_eq!(bob_bal, 25);
        // Bob's wallet now exists
        let wallet = ledger.wallet("bob").expect("wallet should succeed");
        assert_eq!(wallet.total_transferred_in, 25);
    }

    // -- Atomic multi-party settlement --

    #[test]
    fn test_settle_trade_three_party() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint(
                "buyer",
                1000,
                TransactionReason::SystemGrant,
                "initial funds",
            )
            .expect("mint should succeed");

        let (buyer_bal, seller_bal, fee_bal) = ledger
            .settle_trade("buyer", "seller", "platform", 100, 10, "trade-001")
            .expect("settle_trade should succeed");

        assert_eq!(buyer_bal, 900);
        assert_eq!(seller_bal, 90); // 100 - 10 fee
        assert_eq!(fee_bal, 10);
    }

    #[test]
    fn test_settle_trade_zero_fee() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("buyer", 500, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");

        let (buyer_bal, seller_bal, fee_bal) = ledger
            .settle_trade("buyer", "seller", "platform", 200, 0, "trade-002")
            .expect("settle_trade should succeed");

        assert_eq!(buyer_bal, 300);
        assert_eq!(seller_bal, 200);
        assert_eq!(fee_bal, 0);
    }

    #[test]
    fn test_settle_trade_insufficient_funds() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("buyer", 50, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");

        let result = ledger.settle_trade("buyer", "seller", "platform", 100, 10, "trade-003");
        assert!(result.is_err());
        // Verify no partial mutations
        assert_eq!(ledger.balance("buyer").expect("balance should succeed"), 50);
        assert_eq!(ledger.balance("seller").expect("balance should succeed"), 0);
        assert_eq!(
            ledger.balance("platform").expect("balance should succeed"),
            0
        );
    }

    #[test]
    fn test_settle_trade_fee_exceeds_price() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("buyer", 1000, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        let result = ledger.settle_trade("buyer", "seller", "platform", 100, 150, "trade-004");
        assert!(result.is_err());
    }

    #[test]
    fn test_settle_trade_records_reference() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("buyer", 1000, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .settle_trade("buyer", "seller", "platform", 100, 10, "trade-ref-42")
            .expect("settle_trade should succeed");

        let txs = ledger
            .transactions_by_reference("trade-ref-42")
            .expect("transactions_by_reference should succeed");
        assert_eq!(txs.len(), 2); // trade + fee
        assert!(txs.iter().any(|t| t.kind == TransactionKind::Transfer));
        assert!(txs.iter().any(|t| t.kind == TransactionKind::Fee));
    }

    // -- Mint tests --

    #[test]
    fn test_mint_creates_tokens() {
        let (_tmp, ledger) = temp_ledger();
        let balance = ledger
            .mint(
                "agent-1",
                500,
                TransactionReason::HumanTaskInjection,
                "new task from human",
            )
            .expect("mint should succeed");
        assert_eq!(balance, 500);

        let (minted, burned) = ledger
            .mint_burn_summary()
            .expect("mint_burn_summary should succeed");
        assert_eq!(minted, 500);
        assert_eq!(burned, 0);
    }

    #[test]
    fn test_mint_system_grant() {
        let (_tmp, ledger) = temp_ledger();
        let balance = ledger
            .mint("agent-1", 1000, TransactionReason::SystemGrant, "bootstrap")
            .expect("mint should succeed");
        assert_eq!(balance, 1000);
    }

    // -- Burn tests --

    #[test]
    fn test_burn_destroys_tokens() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        let balance = ledger
            .burn(
                "agent-1",
                30,
                TransactionReason::ApiCostSettlement,
                "claude api call",
            )
            .expect("burn should succeed");
        assert_eq!(balance, 70);

        let (minted, burned) = ledger
            .mint_burn_summary()
            .expect("mint_burn_summary should succeed");
        assert_eq!(minted, 100);
        assert_eq!(burned, 30);
    }

    #[test]
    fn test_burn_overdraft_prevented() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 50, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        let result = ledger.burn(
            "agent-1",
            100,
            TransactionReason::ApiCostSettlement,
            "too much",
        );
        assert!(result.is_err());
        assert_eq!(
            ledger.balance("agent-1").expect("balance should succeed"),
            50
        );
    }

    // -- Wallet tests --

    #[test]
    fn test_wallet_stats_accurate() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 200, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .earn("agent-1", 50, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");
        ledger
            .spend("agent-1", 30, TransactionReason::LlmCall, "")
            .expect("spend should succeed");
        ledger
            .transfer(
                "agent-1",
                "agent-2",
                20,
                TransactionReason::PeerTransfer,
                "",
            )
            .expect("transfer should succeed");

        let wallet = ledger.wallet("agent-1").expect("wallet should succeed");
        assert_eq!(wallet.balance, 200); // 200 + 50 - 30 - 20
        assert_eq!(wallet.total_earned, 250); // mint(200) + earn(50)
        assert_eq!(wallet.total_spent, 30);
        assert_eq!(wallet.total_transferred_out, 20);
        assert_eq!(wallet.transaction_count, 4);

        let wallet2 = ledger.wallet("agent-2").expect("wallet should succeed");
        assert_eq!(wallet2.balance, 20);
        assert_eq!(wallet2.total_transferred_in, 20);
    }

    #[test]
    fn test_wallet_default_zero() {
        let (_tmp, ledger) = temp_ledger();
        let wallet = ledger.wallet("nonexistent").expect("wallet should succeed");
        assert_eq!(wallet.balance, 0);
        assert_eq!(wallet.total_earned, 0);
        assert_eq!(wallet.transaction_count, 0);
    }

    // -- All wallets --

    #[test]
    fn test_all_wallets() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-a", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .mint("agent-b", 200, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .mint("agent-c", 50, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");

        let wallets = ledger.all_wallets().expect("all_wallets should succeed");
        assert_eq!(wallets.len(), 3);
        // Ordered by balance DESC
        assert_eq!(wallets[0].agent_id, "agent-b");
        assert_eq!(wallets[1].agent_id, "agent-a");
        assert_eq!(wallets[2].agent_id, "agent-c");
    }

    // -- Total supply --

    #[test]
    fn test_total_supply() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-a", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .mint("agent-b", 200, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .burn("agent-a", 30, TransactionReason::ApiCostSettlement, "")
            .expect("burn should succeed");

        assert_eq!(
            ledger.total_supply().expect("total_supply should succeed"),
            270
        );
    }

    // -- Audit log tests --

    #[test]
    fn test_transactions_for_agent() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .spend("agent-1", 20, TransactionReason::LlmCall, "call 1")
            .expect("spend should succeed");
        ledger
            .spend("agent-1", 10, TransactionReason::ToolUse, "call 2")
            .expect("spend should succeed");

        let txs = ledger
            .transactions_for("agent-1", 100)
            .expect("transactions_for should succeed");
        assert_eq!(txs.len(), 3);
        // Most recent first
        assert_eq!(txs[0].kind, TransactionKind::Spend);
        assert_eq!(txs[0].reason, TransactionReason::ToolUse);
    }

    #[test]
    fn test_transactions_by_kind() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("a", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .mint("b", 200, TransactionReason::HumanTaskInjection, "")
            .expect("mint should succeed");
        ledger
            .earn("a", 50, TransactionReason::TaskCompletion, "")
            .expect("earn should succeed");

        let mints = ledger
            .transactions_by_kind(TransactionKind::Mint, 100)
            .expect("transactions_by_kind should succeed");
        assert_eq!(mints.len(), 2);

        let earns = ledger
            .transactions_by_kind(TransactionKind::Earn, 100)
            .expect("transactions_by_kind should succeed");
        assert_eq!(earns.len(), 1);
    }

    #[test]
    fn test_all_transactions_with_limit() {
        let (_tmp, ledger) = temp_ledger();
        for i in 0..10 {
            ledger
                .mint(
                    "agent",
                    10,
                    TransactionReason::SystemGrant,
                    format!("grant {i}"),
                )
                .expect("operation should succeed");
        }

        let txs = ledger
            .all_transactions(5)
            .expect("all_transactions should succeed");
        assert_eq!(txs.len(), 5);
    }

    #[test]
    fn test_transaction_count() {
        let (_tmp, ledger) = temp_ledger();
        assert_eq!(
            ledger
                .transaction_count()
                .expect("transaction_count should succeed"),
            0
        );

        ledger
            .mint("agent", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        ledger
            .spend("agent", 10, TransactionReason::LlmCall, "")
            .expect("spend should succeed");

        assert_eq!(
            ledger
                .transaction_count()
                .expect("transaction_count should succeed"),
            2
        );
    }

    // -- Serialization roundtrip --

    #[test]
    fn test_transaction_kind_roundtrip() {
        let kinds = [
            TransactionKind::Earn,
            TransactionKind::Spend,
            TransactionKind::Transfer,
            TransactionKind::Mint,
            TransactionKind::Burn,
            TransactionKind::Fee,
        ];
        for kind in &kinds {
            let s = kind.as_str();
            let parsed = TransactionKind::from_str(&s);
            assert_eq!(*kind, parsed);
        }
        // Unknown variant round-trips via its raw value
        let unknown = TransactionKind::Unknown("bogus_kind".to_string());
        let s = unknown.as_str();
        assert_eq!(s, "unknown:bogus_kind");
        // from_str on an unrecognised string produces Unknown
        assert_eq!(
            TransactionKind::from_str("totally_new"),
            TransactionKind::Unknown("totally_new".to_string())
        );
    }

    #[test]
    fn test_transaction_reason_roundtrip() {
        let reasons = [
            TransactionReason::TaskCompletion,
            TransactionReason::MarketplaceSale,
            TransactionReason::ReviewReward,
            TransactionReason::LlmCall,
            TransactionReason::ToolUse,
            TransactionReason::AgentSpawn,
            TransactionReason::IdleCost,
            TransactionReason::MarketplaceTrade,
            TransactionReason::PeerTransfer,
            TransactionReason::HumanTaskInjection,
            TransactionReason::ApiCostSettlement,
            TransactionReason::SystemGrant,
            TransactionReason::PlatformFee,
            TransactionReason::Custom("my_reason".to_string()),
        ];
        for reason in &reasons {
            let s = reason.as_str();
            let parsed = TransactionReason::from_str(&s);
            assert_eq!(*reason, parsed);
        }
    }

    #[test]
    fn test_transaction_json_serde() {
        let tx = Transaction {
            id: "tx-1".to_string(),
            kind: TransactionKind::Earn,
            reason: TransactionReason::TaskCompletion,
            from_agent: None,
            to_agent: Some("agent-1".to_string()),
            amount: 100,
            reference_id: None,
            note: "completed task".to_string(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&tx).expect("should serialize to JSON");
        let parsed: Transaction = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.id, "tx-1");
        assert_eq!(parsed.kind, TransactionKind::Earn);
        assert_eq!(parsed.amount, 100);
    }

    #[test]
    fn test_wallet_json_serde() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        let wallet = ledger.wallet("agent-1").expect("wallet should succeed");
        let json = serde_json::to_string(&wallet).expect("should serialize to JSON");
        let parsed: AgentWallet = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(parsed.agent_id, "agent-1");
        assert_eq!(parsed.balance, 100);
    }

    // -- Concurrent access safety --

    #[test]
    fn test_concurrent_earns_same_agent() {
        let (_tmp, ledger) = temp_ledger();

        // Simulate concurrent access by running many operations sequentially
        // (SQLite's IMMEDIATE transactions ensure serialization)
        for i in 0..100 {
            ledger
                .earn(
                    "agent-1",
                    1,
                    TransactionReason::TaskCompletion,
                    format!("task {i}"),
                )
                .expect("operation should succeed");
        }

        assert_eq!(
            ledger.balance("agent-1").expect("balance should succeed"),
            100
        );
        assert_eq!(
            ledger
                .transaction_count()
                .expect("transaction_count should succeed"),
            100
        );
    }

    #[test]
    fn test_concurrent_spends_overdraft_safe() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("agent-1", 10, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");

        let mut successes = 0;
        let mut failures = 0;
        // Try to spend 1 token 20 times — only 10 should succeed
        for _ in 0..20 {
            match ledger.spend("agent-1", 1, TransactionReason::LlmCall, "") {
                Ok(_) => successes += 1,
                Err(_) => failures += 1,
            }
        }

        assert_eq!(successes, 10);
        assert_eq!(failures, 10);
        assert_eq!(
            ledger.balance("agent-1").expect("balance should succeed"),
            0
        );
    }

    // -- Multi-agent complex scenario --

    #[test]
    fn test_full_economy_lifecycle() {
        let (_tmp, ledger) = temp_ledger();

        // 1. System mints tokens for a new human task
        ledger
            .mint(
                "alice",
                500,
                TransactionReason::HumanTaskInjection,
                "user task #42",
            )
            .expect("mint should succeed");

        // 2. Alice spawns a subagent (costs tokens)
        ledger
            .spend("alice", 50, TransactionReason::AgentSpawn, "spawned bob")
            .expect("spend should succeed");
        ledger
            .mint("bob", 50, TransactionReason::SystemGrant, "birth grant")
            .expect("mint should succeed");

        // 3. Both agents make LLM calls
        ledger
            .spend("alice", 20, TransactionReason::LlmCall, "claude call")
            .expect("spend should succeed");
        ledger
            .spend("bob", 10, TransactionReason::LlmCall, "claude call")
            .expect("spend should succeed");

        // 4. Bob completes a sub-task, earns reward
        ledger
            .earn(
                "bob",
                30,
                TransactionReason::TaskCompletion,
                "sub-task done",
            )
            .expect("earn should succeed");

        // 5. Alice buys a skill from carol via marketplace.
        //    (carol's wallet is auto-created by settle_trade — no zero-mint seed
        //    needed; zero-amount mints are rejected under H1.)
        ledger
            .settle_trade("alice", "carol", "platform", 100, 5, "skill-trade-1")
            .expect("settle_trade should succeed");

        // 6. Burn tokens for API costs
        ledger
            .burn(
                "alice",
                10,
                TransactionReason::ApiCostSettlement,
                "monthly api settle",
            )
            .expect("burn should succeed");

        // Verify final state
        assert_eq!(
            ledger.balance("alice").expect("balance should succeed"),
            320
        ); // 500 - 50 - 20 - 100 - 10
        assert_eq!(ledger.balance("bob").expect("balance should succeed"), 70); // 50 - 10 + 30
        assert_eq!(ledger.balance("carol").expect("balance should succeed"), 95); // 0 + 95 (100 - 5 fee)
        assert_eq!(
            ledger.balance("platform").expect("balance should succeed"),
            5
        ); // fee
        assert_eq!(
            ledger.total_supply().expect("total_supply should succeed"),
            490
        ); // 500 + 50 - 10 - 30 - 10(burn) — wait let me recalc

        // Minted: 500 (alice) + 50 (bob) + 0 (carol) = 550
        // Burned: 10 (alice)
        // Net supply = sum of all balances = 320 + 70 + 95 + 5 = 490
        let (minted, burned) = ledger
            .mint_burn_summary()
            .expect("mint_burn_summary should succeed");
        assert_eq!(minted, 550);
        assert_eq!(burned, 10);
        assert_eq!(
            ledger.total_supply().expect("total_supply should succeed"),
            490
        );

        // Verify audit trail
        let alice_txs = ledger
            .transactions_for("alice", 100)
            .expect("transactions_for should succeed");
        assert!(alice_txs.len() >= 5);

        let all_txs = ledger
            .all_transactions(100)
            .expect("all_transactions should succeed");
        assert!(all_txs.len() >= 9);
    }

    // -- Edge cases --

    #[test]
    fn test_zero_amount_earn() {
        // H1: zero-amount mutations are now rejected at the ledger (previously
        // a permissive no-op). validate_amount enforces amount > 0 everywhere.
        let (_tmp, ledger) = temp_ledger();
        assert!(
            ledger
                .earn("agent-1", 0, TransactionReason::TaskCompletion, "empty task")
                .is_err()
        );
    }

    #[test]
    fn test_zero_amount_transfer() {
        // H1: zero-amount transfer is now rejected at the ledger.
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("alice", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        assert!(
            ledger
                .transfer("alice", "bob", 0, TransactionReason::PeerTransfer, "noop")
                .is_err()
        );
        // Sender balance untouched by the rejected transfer.
        assert_eq!(ledger.balance("alice").expect("bal"), 100);
    }

    #[test]
    fn test_self_transfer() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("alice", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");
        let (from_bal, to_bal) = ledger
            .transfer(
                "alice",
                "alice",
                50,
                TransactionReason::PeerTransfer,
                "self",
            )
            .expect("transfer should succeed");
        // Self-transfer: balance unchanged net
        assert_eq!(from_bal, 100);
        assert_eq!(to_bal, 100);
    }

    #[test]
    fn test_multiple_ledger_instances_same_db() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let db_path = tmp.path().join("shared.db");

        let ledger1 = TokenLedger::new(&db_path).expect("TokenLedger::new should succeed");
        ledger1
            .mint("agent", 100, TransactionReason::SystemGrant, "")
            .expect("mint should succeed");

        // Open second instance pointing to same DB
        let ledger2 = TokenLedger::new(&db_path).expect("TokenLedger::new should succeed");
        assert_eq!(
            ledger2.balance("agent").expect("balance should succeed"),
            100
        );

        ledger2
            .spend("agent", 30, TransactionReason::LlmCall, "")
            .expect("spend should succeed");
        assert_eq!(
            ledger1.balance("agent").expect("balance should succeed"),
            70
        );
    }

    #[test]
    fn test_custom_reason() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn(
                "agent-1",
                10,
                TransactionReason::Custom("bounty_payout".to_string()),
                "bug bounty",
            )
            .expect("operation should succeed");

        let txs = ledger
            .transactions_for("agent-1", 10)
            .expect("transactions_for should succeed");
        assert_eq!(
            txs[0].reason,
            TransactionReason::Custom("bounty_payout".to_string())
        );
    }

    // -- Stake / Unstake tests (C2 regression) --

    #[test]
    fn test_stake_debits_and_records() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 100, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        let (stake_id, bal) = ledger.stake("a", 40, "listing").expect("stake");
        assert_eq!(bal, 60);
        assert_eq!(ledger.balance("a").expect("balance"), 60);
        assert!(!stake_id.is_empty());
    }

    #[test]
    fn test_stake_insufficient_balance_rejected() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 10, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        assert!(ledger.stake("a", 50, "listing").is_err());
        // Balance untouched on failed stake.
        assert_eq!(ledger.balance("a").expect("balance"), 10);
    }

    #[test]
    fn test_unstake_releases_exact_recorded_amount() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 100, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        let (stake_id, _) = ledger.stake("a", 40, "listing").expect("stake");
        let (released, bal) = ledger.unstake("a", &stake_id).expect("unstake");
        assert_eq!(released, 40);
        assert_eq!(bal, 100);
    }

    #[test]
    fn test_unstake_unknown_stake_rejected() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 100, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        // C2 exploit: unstake a fabricated stake_id must NOT mint tokens.
        assert!(ledger.unstake("a", "fabricated-id").is_err());
        assert_eq!(ledger.balance("a").expect("balance"), 100);
    }

    #[test]
    fn test_unstake_double_release_rejected() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 100, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        let (stake_id, _) = ledger.stake("a", 40, "listing").expect("stake");
        ledger.unstake("a", &stake_id).expect("first unstake");
        // Second unstake of the same stake must fail — no double credit.
        assert!(ledger.unstake("a", &stake_id).is_err());
        assert_eq!(ledger.balance("a").expect("balance"), 100);
    }

    #[test]
    fn test_unstake_wrong_owner_rejected() {
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 100, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        let (stake_id, _) = ledger.stake("a", 40, "listing").expect("stake");
        // Attacker "b" cannot release agent "a"'s stake.
        assert!(ledger.unstake("b", &stake_id).is_err());
        assert_eq!(ledger.balance("a").expect("balance"), 60);
    }

    // -- H1: amount-validation / overflow-safety tests --

    #[test]
    fn test_zero_amount_rejected_on_all_paths() {
        let (_tmp, ledger) = temp_ledger();
        assert!(ledger.earn("a", 0, TransactionReason::TaskCompletion, "z").is_err());
        assert!(ledger.mint("a", 0, TransactionReason::TaskCompletion, "z").is_err());
        assert!(ledger.spend("a", 0, TransactionReason::TaskCompletion, "z").is_err());
        assert!(ledger.burn("a", 0, TransactionReason::TaskCompletion, "z").is_err());
        assert!(ledger.transfer("a", "b", 0, TransactionReason::TaskCompletion, "z").is_err());
        assert!(ledger.stake("a", 0, "z").is_err());
    }

    #[test]
    fn test_amount_above_max_rejected() {
        let (_tmp, ledger) = temp_ledger();
        let over = MAX_AMOUNT + 1;
        assert!(ledger.mint("a", over, TransactionReason::TaskCompletion, "big").is_err());
        assert!(ledger.earn("a", over, TransactionReason::TaskCompletion, "big").is_err());
        // A raw u64 > i64::MAX (silent-wrap territory) must also be rejected.
        assert!(
            ledger
                .mint("a", u64::MAX, TransactionReason::TaskCompletion, "wrap")
                .is_err()
        );
        // No wallet should have been created/credited by the rejected mints.
        assert_eq!(ledger.balance("a").expect("balance"), 0);
    }

    #[test]
    fn test_max_amount_boundary_accepted() {
        let (_tmp, ledger) = temp_ledger();
        // Exactly MAX_AMOUNT is allowed.
        ledger
            .mint("a", MAX_AMOUNT, TransactionReason::TaskCompletion, "cap")
            .expect("mint at cap");
        assert_eq!(ledger.balance("a").expect("balance"), MAX_AMOUNT);
    }

    #[test]
    fn test_huge_transfer_amount_rejected_before_mutation() {
        // A transfer of an over-cap amount is rejected up front — the sender is
        // never debited and the recipient wallet is never even created.
        let (_tmp, ledger) = temp_ledger();
        ledger
            .mint("rich", 1000, TransactionReason::TaskCompletion, "seed")
            .expect("mint");
        let r = ledger.transfer(
            "rich",
            "bob",
            MAX_AMOUNT + 1,
            TransactionReason::PeerTransfer,
            "overflow",
        );
        assert!(r.is_err(), "over-cap transfer must be rejected");
        // Sender balance unchanged — validation runs pre-mutation.
        assert_eq!(ledger.balance("rich").expect("bal"), 1000);
    }

    #[test]
    fn test_balance_check_constraint_present() {
        // The v4 migration installs CHECK(balance >= 0). A direct negative
        // write must be rejected by the DB itself, proving the backstop exists.
        let (_tmp, ledger) = temp_ledger();
        ledger
            .earn("a", 10, TransactionReason::TaskCompletion, "seed")
            .expect("earn");
        let conn = ledger.conn().expect("conn");
        let res = conn.execute(
            "UPDATE wallets SET balance = -5 WHERE agent_id = 'a'",
            [],
        );
        assert!(res.is_err(), "CHECK(balance >= 0) must reject negative write");
    }
}
