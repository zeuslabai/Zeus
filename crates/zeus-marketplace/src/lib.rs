//! Agent-to-Agent Skill Marketplace
//!
//! Provides a marketplace for agents to publish, discover, trade, and rate
//! skills/tools. Six core components:
//!
//! 1. **SkillListing** — Published skill with metadata, pricing, ratings
//! 2. **MarketplaceRegistry** — In-memory skill registry with CRUD and search
//! 3. **TradeProtocol** — Negotiation flow: request → offer → execution
//! 4. **TokenLedger** — Agent token balances with atomic debit/credit
//! 5. **ReputationEngine** — Trust scores from trades, reviews, ratings
//! 6. **MarketplaceAPI** — Request/response types for REST integration

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

// ===========================================================================
// Error
// ===========================================================================

#[derive(Debug, thiserror::Error)]
pub enum MarketplaceError {
    #[error("skill not found: {0}")]
    SkillNotFound(String),
    #[error("trade not found: {0}")]
    TradeNotFound(String),
    #[error("insufficient balance: agent {agent_id} has {available}, needs {required}")]
    InsufficientBalance {
        agent_id: String,
        available: u64,
        required: u64,
    },
    #[error("invalid operation: {0}")]
    InvalidOperation(String),
    #[error("duplicate listing: {0}")]
    DuplicateListing(String),
    #[error("agent not found: {0}")]
    AgentNotFound(String),
}

pub type Result<T> = std::result::Result<T, MarketplaceError>;

// ===========================================================================
// 1. SkillListing
// ===========================================================================

/// A skill published on the marketplace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListing {
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher_id: String,
    /// Capabilities this skill provides (e.g., "code_review", "testing").
    pub capabilities: Vec<String>,
    /// Freeform tags for discovery.
    pub tags: Vec<String>,
    /// Price in marketplace tokens. 0 = free.
    pub price: u64,
    /// Semantic version string.
    pub version: String,
    /// Average user rating (0.0 .. 5.0).
    pub rating: f64,
    /// Number of ratings received.
    pub rating_count: u64,
    /// Total times this skill has been acquired.
    pub downloads: u64,
    /// Whether the listing is currently active.
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Arbitrary key-value metadata.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl SkillListing {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        publisher_id: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.into(),
            publisher_id: publisher_id.into(),
            capabilities: Vec::new(),
            tags: Vec::new(),
            price: 0,
            version: "0.1.0".to_string(),
            rating: 0.0,
            rating_count: 0,
            downloads: 0,
            active: true,
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        }
    }

    pub fn with_capabilities(mut self, caps: Vec<String>) -> Self {
        self.capabilities = caps;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_price(mut self, price: u64) -> Self {
        self.price = price;
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Record a new rating (running average).
    pub fn add_rating(&mut self, score: f64) {
        let score = score.clamp(0.0, 5.0);
        let total = self.rating * self.rating_count as f64 + score;
        self.rating_count += 1;
        self.rating = total / self.rating_count as f64;
        self.updated_at = Utc::now();
    }

    /// Increment the download counter.
    pub fn record_download(&mut self) {
        self.downloads += 1;
        self.updated_at = Utc::now();
    }

    /// Check if this skill matches a capability query.
    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }

    /// Check if this skill has a given tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

// ===========================================================================
// 2. MarketplaceRegistry
// ===========================================================================

/// In-memory registry of all skill listings.
pub struct MarketplaceRegistry {
    listings: Arc<RwLock<HashMap<String, SkillListing>>>,
}

impl MarketplaceRegistry {
    pub fn new() -> Self {
        Self {
            listings: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish a new skill listing.
    pub async fn publish(&self, listing: SkillListing) -> Result<String> {
        let mut listings = self.listings.write().await;
        if listings
            .values()
            .any(|l| l.name == listing.name && l.publisher_id == listing.publisher_id && l.active)
        {
            return Err(MarketplaceError::DuplicateListing(listing.name));
        }
        let id = listing.id.clone();
        info!(id = %id, name = %listing.name, "Skill published");
        listings.insert(id.clone(), listing);
        Ok(id)
    }

    /// Remove a listing (soft-delete: marks inactive).
    pub async fn unpublish(&self, id: &str) -> Result<()> {
        let mut listings = self.listings.write().await;
        let listing = listings
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::SkillNotFound(id.to_string()))?;
        listing.active = false;
        listing.updated_at = Utc::now();
        debug!(id, "Skill unpublished");
        Ok(())
    }

    /// Update an existing listing's fields.
    pub async fn update(&self, id: &str, update: SkillUpdate) -> Result<SkillListing> {
        let mut listings = self.listings.write().await;
        let listing = listings
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::SkillNotFound(id.to_string()))?;

        if let Some(name) = update.name {
            listing.name = name;
        }
        if let Some(desc) = update.description {
            listing.description = desc;
        }
        if let Some(price) = update.price {
            listing.price = price;
        }
        if let Some(version) = update.version {
            listing.version = version;
        }
        if let Some(tags) = update.tags {
            listing.tags = tags;
        }
        if let Some(caps) = update.capabilities {
            listing.capabilities = caps;
        }
        listing.updated_at = Utc::now();

        Ok(listing.clone())
    }

    /// Get a single listing by ID.
    pub async fn get(&self, id: &str) -> Result<SkillListing> {
        let listings = self.listings.read().await;
        listings
            .get(id)
            .cloned()
            .ok_or_else(|| MarketplaceError::SkillNotFound(id.to_string()))
    }

    /// List all active listings.
    pub async fn list_active(&self) -> Vec<SkillListing> {
        let listings = self.listings.read().await;
        listings.values().filter(|l| l.active).cloned().collect()
    }

    /// List all listings (including inactive).
    pub async fn list_all(&self) -> Vec<SkillListing> {
        let listings = self.listings.read().await;
        listings.values().cloned().collect()
    }

    /// Search by capability.
    pub async fn search_by_capability(&self, capability: &str) -> Vec<SkillListing> {
        let listings = self.listings.read().await;
        listings
            .values()
            .filter(|l| l.active && l.has_capability(capability))
            .cloned()
            .collect()
    }

    /// Search by tag.
    pub async fn search_by_tag(&self, tag: &str) -> Vec<SkillListing> {
        let listings = self.listings.read().await;
        listings
            .values()
            .filter(|l| l.active && l.has_tag(tag))
            .cloned()
            .collect()
    }

    /// Search by name (case-insensitive substring match).
    pub async fn search_by_name(&self, query: &str) -> Vec<SkillListing> {
        let query_lower = query.to_lowercase();
        let listings = self.listings.read().await;
        listings
            .values()
            .filter(|l| l.active && l.name.to_lowercase().contains(&query_lower))
            .cloned()
            .collect()
    }

    /// Search by publisher.
    pub async fn search_by_publisher(&self, publisher_id: &str) -> Vec<SkillListing> {
        let listings = self.listings.read().await;
        listings
            .values()
            .filter(|l| l.active && l.publisher_id == publisher_id)
            .cloned()
            .collect()
    }

    /// Add a rating to a skill.
    pub async fn rate_skill(&self, id: &str, score: f64) -> Result<f64> {
        let mut listings = self.listings.write().await;
        let listing = listings
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::SkillNotFound(id.to_string()))?;
        listing.add_rating(score);
        Ok(listing.rating)
    }

    /// Record a download/acquisition of a skill.
    pub async fn record_download(&self, id: &str) -> Result<u64> {
        let mut listings = self.listings.write().await;
        let listing = listings
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::SkillNotFound(id.to_string()))?;
        listing.record_download();
        Ok(listing.downloads)
    }

    /// Count of active listings.
    pub async fn active_count(&self) -> usize {
        let listings = self.listings.read().await;
        listings.values().filter(|l| l.active).count()
    }

    /// Total listing count (including inactive).
    pub async fn total_count(&self) -> usize {
        let listings = self.listings.read().await;
        listings.len()
    }
}

impl Default for MarketplaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Partial update for a skill listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub price: Option<u64>,
    pub version: Option<String>,
    pub tags: Option<Vec<String>>,
    pub capabilities: Option<Vec<String>>,
}

// ===========================================================================
// 3. TradeProtocol
// ===========================================================================

/// Status of a trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    Proposed,
    Negotiating,
    Accepted,
    Rejected,
    Completed,
    Cancelled,
}

/// A trade between two agents for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub buyer_id: String,
    pub seller_id: String,
    pub skill_id: String,
    pub offered_price: u64,
    pub final_price: Option<u64>,
    pub status: TradeStatus,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Trade {
    pub fn new(
        buyer_id: impl Into<String>,
        seller_id: impl Into<String>,
        skill_id: impl Into<String>,
        offered_price: u64,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            buyer_id: buyer_id.into(),
            seller_id: seller_id.into(),
            skill_id: skill_id.into(),
            offered_price,
            final_price: None,
            status: TradeStatus::Proposed,
            message: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = msg.into();
        self
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TradeStatus::Completed | TradeStatus::Rejected | TradeStatus::Cancelled
        )
    }
}

/// Manages the lifecycle of trades.
pub struct TradeProtocol {
    trades: Arc<RwLock<HashMap<String, Trade>>>,
}

impl TradeProtocol {
    pub fn new() -> Self {
        Self {
            trades: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Propose a new trade.
    pub async fn propose(&self, trade: Trade) -> Result<String> {
        let id = trade.id.clone();
        let mut trades = self.trades.write().await;
        info!(id = %id, buyer = %trade.buyer_id, seller = %trade.seller_id, "Trade proposed");
        trades.insert(id.clone(), trade);
        Ok(id)
    }

    /// Get a trade by ID.
    pub async fn get(&self, id: &str) -> Result<Trade> {
        let trades = self.trades.read().await;
        trades
            .get(id)
            .cloned()
            .ok_or_else(|| MarketplaceError::TradeNotFound(id.to_string()))
    }

    /// Accept a trade (seller accepts the offered price or specifies a final price).
    pub async fn accept(&self, id: &str, final_price: Option<u64>) -> Result<Trade> {
        let mut trades = self.trades.write().await;
        let trade = trades
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::TradeNotFound(id.to_string()))?;

        if trade.is_terminal() {
            return Err(MarketplaceError::InvalidOperation(format!(
                "trade {id} is already in terminal state {:?}",
                trade.status
            )));
        }

        trade.status = TradeStatus::Accepted;
        trade.final_price = Some(final_price.unwrap_or(trade.offered_price));
        trade.updated_at = Utc::now();
        debug!(id, "Trade accepted");
        Ok(trade.clone())
    }

    /// Reject a trade.
    pub async fn reject(&self, id: &str) -> Result<Trade> {
        let mut trades = self.trades.write().await;
        let trade = trades
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::TradeNotFound(id.to_string()))?;

        if trade.is_terminal() {
            return Err(MarketplaceError::InvalidOperation(format!(
                "trade {id} is already in terminal state {:?}",
                trade.status
            )));
        }

        trade.status = TradeStatus::Rejected;
        trade.updated_at = Utc::now();
        debug!(id, "Trade rejected");
        Ok(trade.clone())
    }

    /// Cancel a trade (by buyer).
    pub async fn cancel(&self, id: &str) -> Result<Trade> {
        let mut trades = self.trades.write().await;
        let trade = trades
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::TradeNotFound(id.to_string()))?;

        if trade.is_terminal() {
            return Err(MarketplaceError::InvalidOperation(format!(
                "trade {id} is already in terminal state {:?}",
                trade.status
            )));
        }

        trade.status = TradeStatus::Cancelled;
        trade.updated_at = Utc::now();
        debug!(id, "Trade cancelled");
        Ok(trade.clone())
    }

    /// Mark a trade as completed (after token transfer).
    pub async fn complete(&self, id: &str) -> Result<Trade> {
        let mut trades = self.trades.write().await;
        let trade = trades
            .get_mut(id)
            .ok_or_else(|| MarketplaceError::TradeNotFound(id.to_string()))?;

        if trade.status != TradeStatus::Accepted {
            return Err(MarketplaceError::InvalidOperation(format!(
                "trade {id} must be accepted before completing, current: {:?}",
                trade.status
            )));
        }

        trade.status = TradeStatus::Completed;
        trade.updated_at = Utc::now();
        info!(id, "Trade completed");
        Ok(trade.clone())
    }

    /// List all trades for an agent (as buyer or seller).
    pub async fn trades_for_agent(&self, agent_id: &str) -> Vec<Trade> {
        let trades = self.trades.read().await;
        trades
            .values()
            .filter(|t| t.buyer_id == agent_id || t.seller_id == agent_id)
            .cloned()
            .collect()
    }

    /// List trades by status.
    pub async fn trades_by_status(&self, status: &TradeStatus) -> Vec<Trade> {
        let trades = self.trades.read().await;
        trades
            .values()
            .filter(|t| t.status == *status)
            .cloned()
            .collect()
    }

    /// Total trade count.
    pub async fn count(&self) -> usize {
        let trades = self.trades.read().await;
        trades.len()
    }
}

impl Default for TradeProtocol {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// 4. TokenLedger
// ===========================================================================

/// A recorded transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub amount: u64,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

/// In-memory token ledger for agent balances.
pub struct TokenLedger {
    balances: Arc<RwLock<HashMap<String, u64>>>,
    transactions: Arc<RwLock<Vec<Transaction>>>,
    max_transactions: usize,
}

impl TokenLedger {
    pub fn new(max_transactions: usize) -> Self {
        Self {
            balances: Arc::new(RwLock::new(HashMap::new())),
            transactions: Arc::new(RwLock::new(Vec::new())),
            max_transactions,
        }
    }

    /// Credit tokens to an agent (mint or reward).
    pub async fn credit(&self, agent_id: &str, amount: u64, reason: impl Into<String>) -> u64 {
        let mut balances = self.balances.write().await;
        let balance = balances.entry(agent_id.to_string()).or_insert(0);
        *balance += amount;
        let new_balance = *balance;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            from: None,
            to: Some(agent_id.to_string()),
            amount,
            reason: reason.into(),
            timestamp: Utc::now(),
        };
        self.append_transaction(tx).await;

        debug!(agent_id, amount, new_balance, "Tokens credited");
        new_balance
    }

    /// Debit tokens from an agent.
    pub async fn debit(
        &self,
        agent_id: &str,
        amount: u64,
        reason: impl Into<String>,
    ) -> Result<u64> {
        let mut balances = self.balances.write().await;
        let balance = balances.entry(agent_id.to_string()).or_insert(0);

        if *balance < amount {
            return Err(MarketplaceError::InsufficientBalance {
                agent_id: agent_id.to_string(),
                available: *balance,
                required: amount,
            });
        }

        *balance -= amount;
        let new_balance = *balance;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            from: Some(agent_id.to_string()),
            to: None,
            amount,
            reason: reason.into(),
            timestamp: Utc::now(),
        };
        self.append_transaction(tx).await;

        debug!(agent_id, amount, new_balance, "Tokens debited");
        Ok(new_balance)
    }

    /// Transfer tokens between agents.
    pub async fn transfer(
        &self,
        from: &str,
        to: &str,
        amount: u64,
        reason: impl Into<String>,
    ) -> Result<()> {
        let reason_str: String = reason.into();
        let mut balances = self.balances.write().await;

        let from_balance = balances.entry(from.to_string()).or_insert(0);
        if *from_balance < amount {
            return Err(MarketplaceError::InsufficientBalance {
                agent_id: from.to_string(),
                available: *from_balance,
                required: amount,
            });
        }
        *from_balance -= amount;

        let to_balance = balances.entry(to.to_string()).or_insert(0);
        *to_balance += amount;

        let tx = Transaction {
            id: Uuid::new_v4().to_string(),
            from: Some(from.to_string()),
            to: Some(to.to_string()),
            amount,
            reason: reason_str,
            timestamp: Utc::now(),
        };
        self.append_transaction(tx).await;

        info!(from, to, amount, "Token transfer");
        Ok(())
    }

    /// Get an agent's balance.
    pub async fn balance(&self, agent_id: &str) -> u64 {
        let balances = self.balances.read().await;
        balances.get(agent_id).copied().unwrap_or(0)
    }

    /// Get all balances.
    pub async fn all_balances(&self) -> HashMap<String, u64> {
        let balances = self.balances.read().await;
        balances.clone()
    }

    /// Get transaction history for an agent.
    pub async fn transactions_for(&self, agent_id: &str) -> Vec<Transaction> {
        let txs = self.transactions.read().await;
        txs.iter()
            .filter(|t| t.from.as_deref() == Some(agent_id) || t.to.as_deref() == Some(agent_id))
            .cloned()
            .collect()
    }

    /// Get all transactions.
    pub async fn all_transactions(&self) -> Vec<Transaction> {
        let txs = self.transactions.read().await;
        txs.clone()
    }

    /// Total supply across all agents.
    pub async fn total_supply(&self) -> u64 {
        let balances = self.balances.read().await;
        balances.values().sum()
    }

    async fn append_transaction(&self, tx: Transaction) {
        let mut txs = self.transactions.write().await;
        txs.push(tx);
        if txs.len() > self.max_transactions {
            txs.remove(0);
        }
    }
}

impl Default for TokenLedger {
    fn default() -> Self {
        Self::new(10_000)
    }
}

// ===========================================================================
// 5. ReputationEngine
// ===========================================================================

/// Per-agent reputation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReputation {
    pub agent_id: String,
    /// Overall trust score (0.0 .. 1.0).
    pub trust_score: f64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub failed_trades: u64,
    /// Average skill rating received (0.0 .. 5.0).
    pub avg_skill_rating: f64,
    pub skill_rating_count: u64,
    /// Average peer review score received (0.0 .. 1.0).
    pub avg_review_score: f64,
    pub review_count: u64,
    pub last_activity: DateTime<Utc>,
}

impl AgentReputation {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            trust_score: 0.5,
            total_trades: 0,
            successful_trades: 0,
            failed_trades: 0,
            avg_skill_rating: 0.0,
            skill_rating_count: 0,
            avg_review_score: 0.0,
            review_count: 0,
            last_activity: Utc::now(),
        }
    }

    /// Trade success rate (0.0 .. 1.0).
    pub fn trade_success_rate(&self) -> f64 {
        if self.total_trades == 0 {
            return 0.0;
        }
        self.successful_trades as f64 / self.total_trades as f64
    }
}

/// Configuration for reputation scoring weights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationConfig {
    /// Weight for trade success rate in trust score (0.0..1.0).
    pub trade_weight: f64,
    /// Weight for skill ratings in trust score (0.0..1.0).
    pub rating_weight: f64,
    /// Weight for peer review scores in trust score (0.0..1.0).
    pub review_weight: f64,
    /// Decay factor per day of inactivity (0.0..1.0). 1.0 = no decay.
    pub inactivity_decay: f64,
}

impl Default for ReputationConfig {
    fn default() -> Self {
        Self {
            trade_weight: 0.4,
            rating_weight: 0.3,
            review_weight: 0.3,
            inactivity_decay: 0.99,
        }
    }
}

/// Manages agent reputation scores.
pub struct ReputationEngine {
    reputations: Arc<RwLock<HashMap<String, AgentReputation>>>,
    config: ReputationConfig,
}

impl ReputationEngine {
    pub fn new(config: ReputationConfig) -> Self {
        Self {
            reputations: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Get or create a reputation record for an agent.
    pub async fn get_or_create(&self, agent_id: &str) -> AgentReputation {
        let mut reps = self.reputations.write().await;
        reps.entry(agent_id.to_string())
            .or_insert_with(|| AgentReputation::new(agent_id))
            .clone()
    }

    /// Get reputation (returns None if never seen).
    pub async fn get(&self, agent_id: &str) -> Option<AgentReputation> {
        let reps = self.reputations.read().await;
        reps.get(agent_id).cloned()
    }

    /// Record a successful trade.
    pub async fn record_trade_success(&self, agent_id: &str) {
        let mut reps = self.reputations.write().await;
        let rep = reps
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentReputation::new(agent_id));
        rep.total_trades += 1;
        rep.successful_trades += 1;
        rep.last_activity = Utc::now();
        self.recalculate_trust(rep);
    }

    /// Record a failed trade.
    pub async fn record_trade_failure(&self, agent_id: &str) {
        let mut reps = self.reputations.write().await;
        let rep = reps
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentReputation::new(agent_id));
        rep.total_trades += 1;
        rep.failed_trades += 1;
        rep.last_activity = Utc::now();
        self.recalculate_trust(rep);
    }

    /// Record a skill rating (0.0 .. 5.0) for an agent's published skill.
    pub async fn record_skill_rating(&self, agent_id: &str, rating: f64) {
        let rating = rating.clamp(0.0, 5.0);
        let mut reps = self.reputations.write().await;
        let rep = reps
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentReputation::new(agent_id));
        let total = rep.avg_skill_rating * rep.skill_rating_count as f64 + rating;
        rep.skill_rating_count += 1;
        rep.avg_skill_rating = total / rep.skill_rating_count as f64;
        rep.last_activity = Utc::now();
        self.recalculate_trust(rep);
    }

    /// Record a peer review score (0.0 .. 1.0) for an agent's work.
    pub async fn record_review_score(&self, agent_id: &str, score: f64) {
        let score = score.clamp(0.0, 1.0);
        let mut reps = self.reputations.write().await;
        let rep = reps
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentReputation::new(agent_id));
        let total = rep.avg_review_score * rep.review_count as f64 + score;
        rep.review_count += 1;
        rep.avg_review_score = total / rep.review_count as f64;
        rep.last_activity = Utc::now();
        self.recalculate_trust(rep);
    }

    /// Apply inactivity decay to all agents.
    pub async fn apply_decay(&self) {
        let now = Utc::now();
        let mut reps = self.reputations.write().await;
        for rep in reps.values_mut() {
            let days_inactive = (now - rep.last_activity).num_days().max(0) as f64;
            if days_inactive > 0.0 {
                let decay = self.config.inactivity_decay.powf(days_inactive);
                rep.trust_score *= decay;
                rep.trust_score = rep.trust_score.max(0.0);
            }
        }
    }

    /// Get trust score for an agent (0.0 if unknown).
    pub async fn trust_score(&self, agent_id: &str) -> f64 {
        let reps = self.reputations.read().await;
        reps.get(agent_id).map(|r| r.trust_score).unwrap_or(0.0)
    }

    /// List top agents by trust score.
    pub async fn top_agents(&self, limit: usize) -> Vec<AgentReputation> {
        let reps = self.reputations.read().await;
        let mut sorted: Vec<AgentReputation> = reps.values().cloned().collect();
        sorted.sort_by(|a, b| {
            b.trust_score
                .partial_cmp(&a.trust_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(limit);
        sorted
    }

    /// Get all reputations.
    pub async fn all_reputations(&self) -> Vec<AgentReputation> {
        let reps = self.reputations.read().await;
        reps.values().cloned().collect()
    }

    /// Recalculate trust score from component metrics.
    fn recalculate_trust(&self, rep: &mut AgentReputation) {
        let trade_component = rep.trade_success_rate();
        let rating_component = if rep.skill_rating_count > 0 {
            rep.avg_skill_rating / 5.0
        } else {
            0.5
        };
        let review_component = if rep.review_count > 0 {
            rep.avg_review_score
        } else {
            0.5
        };

        rep.trust_score = (trade_component * self.config.trade_weight
            + rating_component * self.config.rating_weight
            + review_component * self.config.review_weight)
            .clamp(0.0, 1.0);
    }
}

impl Default for ReputationEngine {
    fn default() -> Self {
        Self::new(ReputationConfig::default())
    }
}

// ===========================================================================
// 6. MarketplaceAPI — Request/Response types
// ===========================================================================

/// Request to publish a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    pub name: String,
    pub description: String,
    pub publisher_id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub price: u64,
    #[serde(default = "default_version")]
    pub version: String,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

/// Request to search skills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: Option<String>,
    pub capability: Option<String>,
    pub tag: Option<String>,
    pub publisher_id: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Request to propose a trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRequest {
    pub buyer_id: String,
    pub skill_id: String,
    pub offered_price: u64,
    #[serde(default)]
    pub message: String,
}

/// Request to rate a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateRequest {
    pub skill_id: String,
    pub agent_id: String,
    pub score: f64,
}

/// Marketplace statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceStats {
    pub total_listings: usize,
    pub active_listings: usize,
    pub total_trades: usize,
    pub completed_trades: usize,
    pub total_token_supply: u64,
    pub total_agents: usize,
}

// ===========================================================================
// 7. Response Types — typed API responses
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListingResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author_agent_id: String,
    pub price_tokens: u64,
    pub rating: f64,
    pub downloads: u64,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl From<SkillListing> for MarketplaceListingResponse {
    fn from(l: SkillListing) -> Self {
        Self {
            id: l.id,
            name: l.name,
            description: l.description,
            author_agent_id: l.publisher_id,
            price_tokens: l.price,
            rating: l.rating,
            downloads: l.downloads,
            tags: l.tags,
            created_at: l.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListResponse {
    pub listings: Vec<MarketplaceListingResponse>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResponse {
    pub skill_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeResponse {
    pub trade_id: String,
    pub status: String,
    pub buyer_id: String,
    pub seller_id: String,
    pub skill_id: String,
    pub price: u64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub amount: u64,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

impl From<Transaction> for TransactionResponse {
    fn from(t: Transaction) -> Self {
        Self {
            id: t.id,
            from: t.from,
            to: t.to,
            amount: t.amount,
            reason: t.reason,
            timestamp: t.timestamp,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerResponse {
    pub agent_id: String,
    pub balance: u64,
    pub transactions: Vec<TransactionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationResponse {
    pub agent_id: String,
    pub score: f64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub ratings: f64,
    pub trade_success_rate: f64,
}

/// The full marketplace coordinator.
pub struct Marketplace {
    pub registry: MarketplaceRegistry,
    pub trades: TradeProtocol,
    pub ledger: TokenLedger,
    pub reputation: ReputationEngine,
}

impl Marketplace {
    pub fn new() -> Self {
        Self {
            registry: MarketplaceRegistry::new(),
            trades: TradeProtocol::new(),
            ledger: TokenLedger::new(10_000),
            reputation: ReputationEngine::default(),
        }
    }

    pub fn with_reputation_config(mut self, config: ReputationConfig) -> Self {
        self.reputation = ReputationEngine::new(config);
        self
    }

    /// End-to-end: execute a trade (accept + transfer tokens + complete + record).
    pub async fn execute_trade(&self, trade_id: &str) -> Result<Trade> {
        let trade = self.trades.accept(trade_id, None).await?;

        let price = trade.final_price.unwrap_or(trade.offered_price);

        // Transfer tokens
        self.ledger
            .transfer(
                &trade.buyer_id,
                &trade.seller_id,
                price,
                format!("trade:{}", trade.id),
            )
            .await?;

        // Record download
        let _ = self.registry.record_download(&trade.skill_id).await;

        // Mark completed
        let completed = self.trades.complete(trade_id).await?;

        // Update reputations
        self.reputation.record_trade_success(&trade.buyer_id).await;
        self.reputation.record_trade_success(&trade.seller_id).await;

        info!(
            trade_id,
            buyer = %completed.buyer_id,
            seller = %completed.seller_id,
            price,
            "Trade executed"
        );

        Ok(completed)
    }

    /// Get marketplace statistics.
    pub async fn stats(&self) -> MarketplaceStats {
        let all_trades = self.trades.count().await;
        let completed = self
            .trades
            .trades_by_status(&TradeStatus::Completed)
            .await
            .len();

        MarketplaceStats {
            total_listings: self.registry.total_count().await,
            active_listings: self.registry.active_count().await,
            total_trades: all_trades,
            completed_trades: completed,
            total_token_supply: self.ledger.total_supply().await,
            total_agents: self.reputation.all_reputations().await.len(),
        }
    }
}

impl Default for Marketplace {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- SkillListing tests -------------------------------------------------

    #[test]
    fn test_skill_listing_new() {
        let s = SkillListing::new("code-review", "Reviews code", "agent-1");
        assert_eq!(s.name, "code-review");
        assert_eq!(s.publisher_id, "agent-1");
        assert!(s.active);
        assert_eq!(s.price, 0);
        assert_eq!(s.version, "0.1.0");
        assert_eq!(s.rating, 0.0);
        assert_eq!(s.downloads, 0);
    }

    #[test]
    fn test_skill_listing_builders() {
        let s = SkillListing::new("test", "desc", "pub1")
            .with_capabilities(vec!["code".into(), "review".into()])
            .with_tags(vec!["rust".into()])
            .with_price(100)
            .with_version("1.0.0")
            .with_metadata("source", "github");
        assert_eq!(s.capabilities.len(), 2);
        assert_eq!(s.tags, vec!["rust"]);
        assert_eq!(s.price, 100);
        assert_eq!(s.version, "1.0.0");
        assert_eq!(s.metadata["source"], "github");
    }

    #[test]
    fn test_skill_listing_add_rating() {
        let mut s = SkillListing::new("t", "d", "p");
        s.add_rating(4.0);
        assert!((s.rating - 4.0).abs() < f64::EPSILON);
        assert_eq!(s.rating_count, 1);

        s.add_rating(2.0);
        assert!((s.rating - 3.0).abs() < f64::EPSILON);
        assert_eq!(s.rating_count, 2);
    }

    #[test]
    fn test_skill_listing_add_rating_clamped() {
        let mut s = SkillListing::new("t", "d", "p");
        s.add_rating(10.0); // should clamp to 5.0
        assert!((s.rating - 5.0).abs() < f64::EPSILON);
        s.add_rating(-5.0); // should clamp to 0.0
        assert!((s.rating - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_skill_listing_record_download() {
        let mut s = SkillListing::new("t", "d", "p");
        s.record_download();
        s.record_download();
        assert_eq!(s.downloads, 2);
    }

    #[test]
    fn test_skill_listing_has_capability() {
        let s =
            SkillListing::new("t", "d", "p").with_capabilities(vec!["code".into(), "test".into()]);
        assert!(s.has_capability("code"));
        assert!(!s.has_capability("deploy"));
    }

    #[test]
    fn test_skill_listing_has_tag() {
        let s = SkillListing::new("t", "d", "p").with_tags(vec!["rust".into()]);
        assert!(s.has_tag("rust"));
        assert!(!s.has_tag("python"));
    }

    #[test]
    fn test_skill_listing_serialization() {
        let s = SkillListing::new("test", "desc", "pub1")
            .with_capabilities(vec!["code".into()])
            .with_price(50);
        let json = serde_json::to_string(&s).expect("should serialize to JSON");
        let de: SkillListing = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "test");
        assert_eq!(de.price, 50);
        assert_eq!(de.capabilities, vec!["code"]);
    }

    #[test]
    fn test_skill_listing_unique_ids() {
        let a = SkillListing::new("t", "d", "p");
        let b = SkillListing::new("t", "d", "p");
        assert_ne!(a.id, b.id);
    }

    // -- MarketplaceRegistry tests ------------------------------------------

    #[tokio::test]
    async fn test_registry_publish_and_get() {
        let reg = MarketplaceRegistry::new();
        let s = SkillListing::new("test", "desc", "pub1");
        let id = s.id.clone();
        reg.publish(s)
            .await
            .expect("async operation should succeed");

        let got = reg.get(&id).await.expect("async operation should succeed");
        assert_eq!(got.name, "test");
    }

    #[tokio::test]
    async fn test_registry_duplicate_listing() {
        let reg = MarketplaceRegistry::new();
        let s1 = SkillListing::new("test", "desc", "pub1");
        reg.publish(s1)
            .await
            .expect("async operation should succeed");

        let s2 = SkillListing::new("test", "desc", "pub1");
        let err = reg.publish(s2).await.unwrap_err();
        assert!(matches!(err, MarketplaceError::DuplicateListing(_)));
    }

    #[tokio::test]
    async fn test_registry_unpublish() {
        let reg = MarketplaceRegistry::new();
        let s = SkillListing::new("test", "desc", "pub1");
        let id = reg
            .publish(s)
            .await
            .expect("async operation should succeed");

        reg.unpublish(&id)
            .await
            .expect("async operation should succeed");
        let got = reg.get(&id).await.expect("async operation should succeed");
        assert!(!got.active);
        assert_eq!(reg.active_count().await, 0);
    }

    #[tokio::test]
    async fn test_registry_update() {
        let reg = MarketplaceRegistry::new();
        let s = SkillListing::new("old-name", "desc", "pub1");
        let id = reg
            .publish(s)
            .await
            .expect("async operation should succeed");

        let updated = reg
            .update(
                &id,
                SkillUpdate {
                    name: Some("new-name".into()),
                    price: Some(200),
                    ..Default::default()
                },
            )
            .await
            .expect("async operation should succeed");
        assert_eq!(updated.name, "new-name");
        assert_eq!(updated.price, 200);
    }

    #[tokio::test]
    async fn test_registry_search_by_capability() {
        let reg = MarketplaceRegistry::new();
        reg.publish(SkillListing::new("a", "d", "p").with_capabilities(vec!["code".into()]))
            .await
            .expect("SkillListing::new should succeed");
        reg.publish(SkillListing::new("b", "d", "p2").with_capabilities(vec!["test".into()]))
            .await
            .expect("SkillListing::new should succeed");

        let results = reg.search_by_capability("code").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "a");
    }

    #[tokio::test]
    async fn test_registry_search_by_tag() {
        let reg = MarketplaceRegistry::new();
        reg.publish(SkillListing::new("a", "d", "p").with_tags(vec!["rust".into(), "fast".into()]))
            .await
            .expect("SkillListing::new should succeed");
        reg.publish(SkillListing::new("b", "d", "p2").with_tags(vec!["python".into()]))
            .await
            .expect("SkillListing::new should succeed");

        let results = reg.search_by_tag("rust").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "a");
    }

    #[tokio::test]
    async fn test_registry_search_by_name() {
        let reg = MarketplaceRegistry::new();
        reg.publish(SkillListing::new("Code Review Pro", "d", "p"))
            .await
            .expect("SkillListing::new should succeed");
        reg.publish(SkillListing::new("Test Runner", "d", "p2"))
            .await
            .expect("SkillListing::new should succeed");

        let results = reg.search_by_name("code").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Code Review Pro");
    }

    #[tokio::test]
    async fn test_registry_search_by_publisher() {
        let reg = MarketplaceRegistry::new();
        reg.publish(SkillListing::new("a", "d", "pub1"))
            .await
            .expect("SkillListing::new should succeed");
        reg.publish(SkillListing::new("b", "d", "pub2"))
            .await
            .expect("SkillListing::new should succeed");

        let results = reg.search_by_publisher("pub1").await;
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_registry_rate_skill() {
        let reg = MarketplaceRegistry::new();
        let s = SkillListing::new("t", "d", "p");
        let id = reg
            .publish(s)
            .await
            .expect("async operation should succeed");

        reg.rate_skill(&id, 4.0)
            .await
            .expect("async operation should succeed");
        reg.rate_skill(&id, 2.0)
            .await
            .expect("async operation should succeed");

        let got = reg.get(&id).await.expect("async operation should succeed");
        assert!((got.rating - 3.0).abs() < f64::EPSILON);
        assert_eq!(got.rating_count, 2);
    }

    #[tokio::test]
    async fn test_registry_counts() {
        let reg = MarketplaceRegistry::new();
        reg.publish(SkillListing::new("a", "d", "p"))
            .await
            .expect("SkillListing::new should succeed");
        let id = reg
            .publish(SkillListing::new("b", "d", "p2"))
            .await
            .expect("SkillListing::new should succeed");
        reg.unpublish(&id)
            .await
            .expect("async operation should succeed");

        assert_eq!(reg.active_count().await, 1);
        assert_eq!(reg.total_count().await, 2);
    }

    #[tokio::test]
    async fn test_registry_not_found() {
        let reg = MarketplaceRegistry::new();
        let err = reg.get("nonexistent").await.unwrap_err();
        assert!(matches!(err, MarketplaceError::SkillNotFound(_)));
    }

    // -- TradeProtocol tests ------------------------------------------------

    #[tokio::test]
    async fn test_trade_propose_and_get() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("buyer", "seller", "skill-1", 100);
        let id = trade.id.clone();
        tp.propose(trade)
            .await
            .expect("async operation should succeed");

        let got = tp.get(&id).await.expect("async operation should succeed");
        assert_eq!(got.buyer_id, "buyer");
        assert_eq!(got.status, TradeStatus::Proposed);
    }

    #[tokio::test]
    async fn test_trade_accept() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let accepted = tp
            .accept(&id, None)
            .await
            .expect("async operation should succeed");
        assert_eq!(accepted.status, TradeStatus::Accepted);
        assert_eq!(accepted.final_price, Some(100));
    }

    #[tokio::test]
    async fn test_trade_accept_with_counter_price() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let accepted = tp
            .accept(&id, Some(80))
            .await
            .expect("async operation should succeed");
        assert_eq!(accepted.final_price, Some(80));
    }

    #[tokio::test]
    async fn test_trade_reject() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let rejected = tp
            .reject(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(rejected.status, TradeStatus::Rejected);
    }

    #[tokio::test]
    async fn test_trade_cancel() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let cancelled = tp
            .cancel(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(cancelled.status, TradeStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_trade_complete_requires_accepted() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let err = tp.complete(&id).await.unwrap_err();
        assert!(matches!(err, MarketplaceError::InvalidOperation(_)));
    }

    #[tokio::test]
    async fn test_trade_accept_reject_complete_flow() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");

        tp.accept(&id, None)
            .await
            .expect("async operation should succeed");
        tp.complete(&id)
            .await
            .expect("async operation should succeed");

        let got = tp.get(&id).await.expect("async operation should succeed");
        assert_eq!(got.status, TradeStatus::Completed);
    }

    #[tokio::test]
    async fn test_trade_terminal_state_prevents_changes() {
        let tp = TradeProtocol::new();
        let trade = Trade::new("b", "s", "sk", 100);
        let id = tp
            .propose(trade)
            .await
            .expect("async operation should succeed");
        tp.reject(&id)
            .await
            .expect("async operation should succeed");

        // Can't accept a rejected trade
        let err = tp.accept(&id, None).await.unwrap_err();
        assert!(matches!(err, MarketplaceError::InvalidOperation(_)));
    }

    #[tokio::test]
    async fn test_trade_for_agent() {
        let tp = TradeProtocol::new();
        tp.propose(Trade::new("buyer", "seller1", "s1", 10))
            .await
            .expect("Trade::new should succeed");
        tp.propose(Trade::new("buyer", "seller2", "s2", 20))
            .await
            .expect("Trade::new should succeed");
        tp.propose(Trade::new("other", "seller1", "s3", 30))
            .await
            .expect("Trade::new should succeed");

        let buyer_trades = tp.trades_for_agent("buyer").await;
        assert_eq!(buyer_trades.len(), 2);

        let seller1_trades = tp.trades_for_agent("seller1").await;
        assert_eq!(seller1_trades.len(), 2);
    }

    #[tokio::test]
    async fn test_trade_by_status() {
        let tp = TradeProtocol::new();
        let t1 = tp
            .propose(Trade::new("b", "s", "sk", 10))
            .await
            .expect("Trade::new should succeed");
        tp.propose(Trade::new("b", "s2", "sk2", 20))
            .await
            .expect("Trade::new should succeed");
        tp.reject(&t1)
            .await
            .expect("async operation should succeed");

        let proposed = tp.trades_by_status(&TradeStatus::Proposed).await;
        assert_eq!(proposed.len(), 1);
        let rejected = tp.trades_by_status(&TradeStatus::Rejected).await;
        assert_eq!(rejected.len(), 1);
    }

    #[tokio::test]
    async fn test_trade_serialization() {
        let trade = Trade::new("b", "s", "sk", 100).with_message("please sell");
        let json = serde_json::to_string(&trade).expect("should serialize to JSON");
        let de: Trade = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.buyer_id, "b");
        assert_eq!(de.message, "please sell");
        assert_eq!(de.status, TradeStatus::Proposed);
    }

    #[tokio::test]
    async fn test_trade_not_found() {
        let tp = TradeProtocol::new();
        let err = tp.get("nonexistent").await.unwrap_err();
        assert!(matches!(err, MarketplaceError::TradeNotFound(_)));
    }

    #[test]
    fn test_trade_status_serialization() {
        let statuses = vec![
            TradeStatus::Proposed,
            TradeStatus::Negotiating,
            TradeStatus::Accepted,
            TradeStatus::Rejected,
            TradeStatus::Completed,
            TradeStatus::Cancelled,
        ];
        for s in &statuses {
            let json = serde_json::to_string(s).expect("should serialize to JSON");
            let de: TradeStatus = serde_json::from_str(&json).expect("should parse successfully");
            assert_eq!(&de, s);
        }
    }

    // -- TokenLedger tests --------------------------------------------------

    #[tokio::test]
    async fn test_ledger_credit() {
        let ledger = TokenLedger::default();
        let balance = ledger.credit("agent-1", 100, "initial").await;
        assert_eq!(balance, 100);

        let balance2 = ledger.credit("agent-1", 50, "bonus").await;
        assert_eq!(balance2, 150);
    }

    #[tokio::test]
    async fn test_ledger_debit() {
        let ledger = TokenLedger::default();
        ledger.credit("agent-1", 100, "initial").await;

        let balance = ledger
            .debit("agent-1", 40, "purchase")
            .await
            .expect("async operation should succeed");
        assert_eq!(balance, 60);
    }

    #[tokio::test]
    async fn test_ledger_debit_insufficient() {
        let ledger = TokenLedger::default();
        ledger.credit("agent-1", 50, "initial").await;

        let err = ledger.debit("agent-1", 100, "purchase").await.unwrap_err();
        assert!(matches!(
            err,
            MarketplaceError::InsufficientBalance {
                available: 50,
                required: 100,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn test_ledger_transfer() {
        let ledger = TokenLedger::default();
        ledger.credit("buyer", 100, "initial").await;

        ledger
            .transfer("buyer", "seller", 60, "trade")
            .await
            .expect("async operation should succeed");

        assert_eq!(ledger.balance("buyer").await, 40);
        assert_eq!(ledger.balance("seller").await, 60);
    }

    #[tokio::test]
    async fn test_ledger_transfer_insufficient() {
        let ledger = TokenLedger::default();
        ledger.credit("buyer", 30, "initial").await;

        let err = ledger
            .transfer("buyer", "seller", 50, "trade")
            .await
            .unwrap_err();
        assert!(matches!(err, MarketplaceError::InsufficientBalance { .. }));
    }

    #[tokio::test]
    async fn test_ledger_balance_unknown_agent() {
        let ledger = TokenLedger::default();
        assert_eq!(ledger.balance("unknown").await, 0);
    }

    #[tokio::test]
    async fn test_ledger_total_supply() {
        let ledger = TokenLedger::default();
        ledger.credit("a1", 100, "").await;
        ledger.credit("a2", 200, "").await;

        assert_eq!(ledger.total_supply().await, 300);

        // Transfer doesn't change total supply
        ledger
            .transfer("a1", "a2", 50, "")
            .await
            .expect("async operation should succeed");
        assert_eq!(ledger.total_supply().await, 300);
    }

    #[tokio::test]
    async fn test_ledger_transactions() {
        let ledger = TokenLedger::default();
        ledger.credit("a1", 100, "mint").await;
        ledger
            .debit("a1", 30, "spend")
            .await
            .expect("async operation should succeed");

        let txs = ledger.transactions_for("a1").await;
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].amount, 100);
        assert_eq!(txs[1].amount, 30);
    }

    #[tokio::test]
    async fn test_ledger_transaction_cap() {
        let ledger = TokenLedger::new(3);
        for i in 0..5 {
            ledger.credit("a", 1, format!("tx-{i}")).await;
        }
        let txs = ledger.all_transactions().await;
        assert_eq!(txs.len(), 3);
        // Oldest entries should have been dropped
        assert_eq!(txs[0].reason, "tx-2");
    }

    #[tokio::test]
    async fn test_ledger_all_balances() {
        let ledger = TokenLedger::default();
        ledger.credit("a1", 100, "").await;
        ledger.credit("a2", 200, "").await;

        let balances = ledger.all_balances().await;
        assert_eq!(balances.len(), 2);
        assert_eq!(balances["a1"], 100);
        assert_eq!(balances["a2"], 200);
    }

    #[test]
    fn test_transaction_serialization() {
        let tx = Transaction {
            id: "tx-1".into(),
            from: Some("a".into()),
            to: Some("b".into()),
            amount: 100,
            reason: "trade".into(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&tx).expect("should serialize to JSON");
        let de: Transaction = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.amount, 100);
        assert_eq!(de.from, Some("a".into()));
    }

    // -- ReputationEngine tests ---------------------------------------------

    #[tokio::test]
    async fn test_reputation_new_agent() {
        let engine = ReputationEngine::default();
        let rep = engine.get_or_create("agent-1").await;
        assert_eq!(rep.agent_id, "agent-1");
        assert!((rep.trust_score - 0.5).abs() < f64::EPSILON);
        assert_eq!(rep.total_trades, 0);
    }

    #[tokio::test]
    async fn test_reputation_trade_success() {
        let engine = ReputationEngine::default();
        engine.record_trade_success("agent-1").await;
        engine.record_trade_success("agent-1").await;

        let rep = engine
            .get("agent-1")
            .await
            .expect("async operation should succeed");
        assert_eq!(rep.total_trades, 2);
        assert_eq!(rep.successful_trades, 2);
        assert!((rep.trade_success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_reputation_trade_failure() {
        let engine = ReputationEngine::default();
        engine.record_trade_success("a").await;
        engine.record_trade_failure("a").await;

        let rep = engine
            .get("a")
            .await
            .expect("async operation should succeed");
        assert_eq!(rep.total_trades, 2);
        assert_eq!(rep.successful_trades, 1);
        assert_eq!(rep.failed_trades, 1);
        assert!((rep.trade_success_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_reputation_skill_rating() {
        let engine = ReputationEngine::default();
        engine.record_skill_rating("a", 4.0).await;
        engine.record_skill_rating("a", 2.0).await;

        let rep = engine
            .get("a")
            .await
            .expect("async operation should succeed");
        assert!((rep.avg_skill_rating - 3.0).abs() < f64::EPSILON);
        assert_eq!(rep.skill_rating_count, 2);
    }

    #[tokio::test]
    async fn test_reputation_review_score() {
        let engine = ReputationEngine::default();
        engine.record_review_score("a", 0.9).await;
        engine.record_review_score("a", 0.7).await;

        let rep = engine
            .get("a")
            .await
            .expect("async operation should succeed");
        assert!((rep.avg_review_score - 0.8).abs() < f64::EPSILON);
        assert_eq!(rep.review_count, 2);
    }

    #[tokio::test]
    async fn test_reputation_trust_score_recalculation() {
        let config = ReputationConfig {
            trade_weight: 0.5,
            rating_weight: 0.25,
            review_weight: 0.25,
            inactivity_decay: 0.99,
        };
        let engine = ReputationEngine::new(config);

        // All perfect scores
        engine.record_trade_success("a").await;
        engine.record_skill_rating("a", 5.0).await;
        engine.record_review_score("a", 1.0).await;

        let rep = engine
            .get("a")
            .await
            .expect("async operation should succeed");
        // trade: 1.0 * 0.5 = 0.5
        // rating: (5.0/5.0) * 0.25 = 0.25
        // review: 1.0 * 0.25 = 0.25
        // total = 1.0
        assert!((rep.trust_score - 1.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_reputation_top_agents() {
        let engine = ReputationEngine::default();
        engine.record_trade_success("good").await;
        engine.record_trade_success("good").await;
        engine.record_trade_failure("bad").await;

        let top = engine.top_agents(10).await;
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].agent_id, "good");
    }

    #[tokio::test]
    async fn test_reputation_unknown_agent() {
        let engine = ReputationEngine::default();
        assert!(engine.get("unknown").await.is_none());
        assert!((engine.trust_score("unknown").await - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reputation_serialization() {
        let rep = AgentReputation::new("a");
        let json = serde_json::to_string(&rep).expect("should serialize to JSON");
        let de: AgentReputation = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.agent_id, "a");
        assert!((de.trust_score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reputation_config_serialization() {
        let config = ReputationConfig::default();
        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let de: ReputationConfig = serde_json::from_str(&json).expect("should parse successfully");
        assert!((de.trade_weight - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_trade_success_rate_zero_trades() {
        let rep = AgentReputation::new("a");
        assert!((rep.trade_success_rate() - 0.0).abs() < f64::EPSILON);
    }

    // -- MarketplaceAPI types tests -----------------------------------------

    #[test]
    fn test_publish_request_serialization() {
        let req = PublishRequest {
            name: "code-review".into(),
            description: "Reviews code".into(),
            publisher_id: "agent-1".into(),
            capabilities: vec!["review".into()],
            tags: vec!["rust".into()],
            price: 50,
            version: "1.0.0".into(),
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: PublishRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "code-review");
        assert_eq!(de.price, 50);
    }

    #[test]
    fn test_search_request_serialization() {
        let req = SearchRequest {
            query: Some("code".into()),
            capability: None,
            tag: Some("rust".into()),
            publisher_id: None,
            limit: Some(10),
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: SearchRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.query, Some("code".into()));
    }

    #[test]
    fn test_trade_request_serialization() {
        let req = TradeRequest {
            buyer_id: "buyer".into(),
            skill_id: "skill-1".into(),
            offered_price: 100,
            message: "want this".into(),
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: TradeRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.offered_price, 100);
    }

    #[test]
    fn test_rate_request_serialization() {
        let req = RateRequest {
            skill_id: "s1".into(),
            agent_id: "a1".into(),
            score: 4.5,
        };
        let json = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: RateRequest = serde_json::from_str(&json).expect("should parse successfully");
        assert!((de.score - 4.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_marketplace_stats_serialization() {
        let stats = MarketplaceStats {
            total_listings: 10,
            active_listings: 8,
            total_trades: 50,
            completed_trades: 45,
            total_token_supply: 10000,
            total_agents: 5,
        };
        let json = serde_json::to_string(&stats).expect("should serialize to JSON");
        let de: MarketplaceStats = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.total_listings, 10);
        assert_eq!(de.completed_trades, 45);
    }

    // -- Marketplace integration tests --------------------------------------

    #[tokio::test]
    async fn test_marketplace_full_trade_flow() {
        let mp = Marketplace::new();

        // Publish a skill
        let skill = SkillListing::new("code-review", "Reviews code", "seller")
            .with_price(50)
            .with_capabilities(vec!["review".into()]);
        let skill_id = mp
            .registry
            .publish(skill)
            .await
            .expect("async operation should succeed");

        // Fund the buyer
        mp.ledger.credit("buyer", 1000, "initial").await;

        // Propose a trade
        let trade = Trade::new("buyer", "seller", &skill_id, 50);
        let trade_id = mp
            .trades
            .propose(trade)
            .await
            .expect("async operation should succeed");

        // Execute the trade
        let completed = mp
            .execute_trade(&trade_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(completed.status, TradeStatus::Completed);

        // Verify balances
        assert_eq!(mp.ledger.balance("buyer").await, 950);
        assert_eq!(mp.ledger.balance("seller").await, 50);

        // Verify download recorded
        let skill = mp
            .registry
            .get(&skill_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(skill.downloads, 1);

        // Verify reputations updated
        let buyer_rep = mp
            .reputation
            .get("buyer")
            .await
            .expect("async operation should succeed");
        assert_eq!(buyer_rep.successful_trades, 1);
        let seller_rep = mp
            .reputation
            .get("seller")
            .await
            .expect("async operation should succeed");
        assert_eq!(seller_rep.successful_trades, 1);
    }

    #[tokio::test]
    async fn test_marketplace_trade_insufficient_funds() {
        let mp = Marketplace::new();

        let skill = SkillListing::new("expensive", "d", "seller").with_price(1000);
        let skill_id = mp
            .registry
            .publish(skill)
            .await
            .expect("async operation should succeed");

        mp.ledger.credit("buyer", 10, "initial").await;

        let trade = Trade::new("buyer", "seller", &skill_id, 1000);
        let trade_id = mp
            .trades
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let err = mp.execute_trade(&trade_id).await.unwrap_err();
        assert!(matches!(err, MarketplaceError::InsufficientBalance { .. }));
    }

    #[tokio::test]
    async fn test_marketplace_stats() {
        let mp = Marketplace::new();
        mp.registry
            .publish(SkillListing::new("a", "d", "p"))
            .await
            .expect("SkillListing::new should succeed");
        mp.ledger.credit("x", 500, "").await;

        let stats = mp.stats().await;
        assert_eq!(stats.total_listings, 1);
        assert_eq!(stats.active_listings, 1);
        assert_eq!(stats.total_token_supply, 500);
    }

    #[tokio::test]
    async fn test_marketplace_free_trade() {
        let mp = Marketplace::new();

        let skill = SkillListing::new("free-tool", "d", "seller").with_price(0);
        let skill_id = mp
            .registry
            .publish(skill)
            .await
            .expect("async operation should succeed");

        // No need to fund buyer for free skill
        let trade = Trade::new("buyer", "seller", &skill_id, 0);
        let trade_id = mp
            .trades
            .propose(trade)
            .await
            .expect("async operation should succeed");

        let completed = mp
            .execute_trade(&trade_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(completed.status, TradeStatus::Completed);
        assert_eq!(mp.ledger.balance("buyer").await, 0);
        assert_eq!(mp.ledger.balance("seller").await, 0);
    }

    // -- Error display tests ------------------------------------------------

    #[test]
    fn test_error_display() {
        let errors = vec![
            MarketplaceError::SkillNotFound("s1".into()),
            MarketplaceError::TradeNotFound("t1".into()),
            MarketplaceError::InsufficientBalance {
                agent_id: "a".into(),
                available: 10,
                required: 100,
            },
            MarketplaceError::InvalidOperation("bad".into()),
            MarketplaceError::DuplicateListing("dup".into()),
            MarketplaceError::AgentNotFound("a".into()),
        ];
        for err in &errors {
            let msg = format!("{err}");
            assert!(!msg.is_empty());
        }
    }

    #[test]
    fn test_skill_update_default() {
        let update = SkillUpdate::default();
        assert!(update.name.is_none());
        assert!(update.price.is_none());
    }

    // -- Response type tests ---------------------------------------------------

    #[test]
    fn test_listing_response_from_skill_listing() {
        let skill = SkillListing::new("code-review", "Reviews code", "agent-1")
            .with_price(100)
            .with_tags(vec!["rust".into(), "review".into()]);
        let created = skill.created_at;
        let resp: MarketplaceListingResponse = skill.into();
        assert_eq!(resp.id.len(), 36); // UUID
        assert_eq!(resp.name, "code-review");
        assert_eq!(resp.description, "Reviews code");
        assert_eq!(resp.author_agent_id, "agent-1");
        assert_eq!(resp.price_tokens, 100);
        assert_eq!(resp.rating, 0.0);
        assert_eq!(resp.downloads, 0);
        assert_eq!(resp.tags, vec!["rust", "review"]);
        assert_eq!(resp.created_at, created);
    }

    #[test]
    fn test_transaction_response_from_transaction() {
        let tx = Transaction {
            id: "tx-1".into(),
            from: Some("alice".into()),
            to: Some("bob".into()),
            amount: 42,
            reason: "trade".into(),
            timestamp: Utc::now(),
        };
        let ts = tx.timestamp;
        let resp: TransactionResponse = tx.into();
        assert_eq!(resp.id, "tx-1");
        assert_eq!(resp.from, Some("alice".into()));
        assert_eq!(resp.to, Some("bob".into()));
        assert_eq!(resp.amount, 42);
        assert_eq!(resp.reason, "trade");
        assert_eq!(resp.timestamp, ts);
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        // MarketplaceListingResponse
        let listing_resp = MarketplaceListingResponse {
            id: "id-1".into(),
            name: "skill".into(),
            description: "desc".into(),
            author_agent_id: "agent".into(),
            price_tokens: 50,
            rating: 4.5,
            downloads: 10,
            tags: vec!["tag".into()],
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&listing_resp).expect("should serialize to JSON");
        let de: MarketplaceListingResponse =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "skill");
        assert_eq!(de.price_tokens, 50);

        // PublishResponse
        let pub_resp = PublishResponse {
            skill_id: "s1".into(),
            status: "published".into(),
        };
        let json = serde_json::to_string(&pub_resp).expect("should serialize to JSON");
        let de: PublishResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.skill_id, "s1");
        assert_eq!(de.status, "published");

        // TradeResponse
        let trade_resp = TradeResponse {
            trade_id: "t1".into(),
            status: "proposed".into(),
            buyer_id: "b".into(),
            seller_id: "s".into(),
            skill_id: "sk".into(),
            price: 100,
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&trade_resp).expect("should serialize to JSON");
        let de: TradeResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.trade_id, "t1");
        assert_eq!(de.price, 100);

        // LedgerResponse
        let ledger_resp = LedgerResponse {
            agent_id: "a1".into(),
            balance: 500,
            transactions: vec![],
        };
        let json = serde_json::to_string(&ledger_resp).expect("should serialize to JSON");
        let de: LedgerResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.balance, 500);

        // ReputationResponse
        let rep_resp = ReputationResponse {
            agent_id: "a1".into(),
            score: 0.8,
            total_trades: 10,
            successful_trades: 9,
            ratings: 4.2,
            trade_success_rate: 0.9,
        };
        let json = serde_json::to_string(&rep_resp).expect("should serialize to JSON");
        let de: ReputationResponse =
            serde_json::from_str(&json).expect("should parse successfully");
        assert!((de.score - 0.8).abs() < f64::EPSILON);
        assert_eq!(de.total_trades, 10);
    }
}
