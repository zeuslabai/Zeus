//! SQLite-backed persistence for the Agora Marketplace.
//!
//! Mirrors the `PantheonStore` pattern: `Arc<Mutex<Connection>>` with WAL mode.
//! Provides CRUD for skill listings, trades, token ledger, and ratings.

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;
// ============================================================================
// MarketplaceStore
// ============================================================================

/// Versioned schema migrations for the Marketplace SQLite database.
const MARKETPLACE_MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    "CREATE TABLE IF NOT EXISTS skill_listings (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT NOT NULL,
        publisher_id TEXT NOT NULL,
        capabilities TEXT NOT NULL DEFAULT '[]',
        tags TEXT NOT NULL DEFAULT '[]',
        price INTEGER NOT NULL DEFAULT 0,
        version TEXT NOT NULL DEFAULT '0.1.0',
        rating REAL NOT NULL DEFAULT 0.0,
        rating_count INTEGER NOT NULL DEFAULT 0,
        downloads INTEGER NOT NULL DEFAULT 0,
        active INTEGER NOT NULL DEFAULT 1,
        source TEXT NOT NULL DEFAULT 'local',
        metadata TEXT NOT NULL DEFAULT '{}',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS trades (
        id TEXT PRIMARY KEY,
        buyer_id TEXT NOT NULL,
        seller_id TEXT NOT NULL,
        skill_id TEXT NOT NULL,
        offered_price INTEGER NOT NULL,
        final_price INTEGER,
        status TEXT NOT NULL DEFAULT 'proposed',
        message TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS token_balances (
        agent_id TEXT PRIMARY KEY,
        balance INTEGER NOT NULL DEFAULT 0,
        total_earned INTEGER NOT NULL DEFAULT 0,
        total_spent INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS token_transactions (
        id TEXT PRIMARY KEY,
        from_agent TEXT,
        to_agent TEXT,
        amount INTEGER NOT NULL,
        reason TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS agent_reputation (
        agent_id TEXT PRIMARY KEY,
        trust_score REAL NOT NULL DEFAULT 0.5,
        total_trades INTEGER NOT NULL DEFAULT 0,
        successful_trades INTEGER NOT NULL DEFAULT 0,
        failed_trades INTEGER NOT NULL DEFAULT 0,
        avg_skill_rating REAL NOT NULL DEFAULT 0.0,
        skill_rating_count INTEGER NOT NULL DEFAULT 0,
        last_activity TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS skill_ratings (
        id TEXT PRIMARY KEY,
        skill_id TEXT NOT NULL,
        reviewer_id TEXT NOT NULL,
        reviewer_name TEXT NOT NULL,
        score REAL NOT NULL,
        review_text TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS bounties (
        id TEXT PRIMARY KEY,
        poster_id TEXT NOT NULL,
        title TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        reward_credits INTEGER NOT NULL,
        skill_tags TEXT NOT NULL DEFAULT '[]',
        deadline TEXT,
        status TEXT NOT NULL DEFAULT 'open',
        claimer_id TEXT,
        claimed_at TEXT,
        completed_at TEXT,
        verifier_id TEXT,
        verified_at TEXT,
        room_id TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS agent_teams (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        members TEXT NOT NULL DEFAULT '[]',
        split_pct TEXT NOT NULL DEFAULT '[]',
        wallet_id TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_listings_active ON skill_listings(active);
    CREATE INDEX IF NOT EXISTS idx_listings_publisher ON skill_listings(publisher_id);
    CREATE INDEX IF NOT EXISTS idx_listings_source ON skill_listings(source);
    CREATE INDEX IF NOT EXISTS idx_trades_buyer ON trades(buyer_id);
    CREATE INDEX IF NOT EXISTS idx_trades_seller ON trades(seller_id);
    CREATE INDEX IF NOT EXISTS idx_trades_status ON trades(status);
    CREATE INDEX IF NOT EXISTS idx_token_txns_from ON token_transactions(from_agent);
    CREATE INDEX IF NOT EXISTS idx_token_txns_to ON token_transactions(to_agent);
    CREATE INDEX IF NOT EXISTS idx_ratings_skill ON skill_ratings(skill_id);
    CREATE INDEX IF NOT EXISTS idx_bounties_poster ON bounties(poster_id);
    CREATE INDEX IF NOT EXISTS idx_bounties_status ON bounties(status);
    CREATE INDEX IF NOT EXISTS idx_bounties_claimer ON bounties(claimer_id);",
];

#[derive(Clone)]
pub struct MarketplaceStore {
    db: Arc<Mutex<Connection>>,
}

impl MarketplaceStore {
    /// Open (or create) the marketplace SQLite database.
    pub fn new(db_path: &PathBuf) -> Result<Self, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open marketplace db: {}", e))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| format!("Failed to set pragmas: {}", e))?;

        crate::db::run_migrations(&conn, MARKETPLACE_MIGRATIONS)
            .map_err(|e| format!("Marketplace schema migration failed: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory marketplace store (for fallback / tests).
    pub fn in_memory() -> Result<Self, String> {
        let path = PathBuf::from(":memory:");
        Self::new(&path)
    }

    // ── Skill Listings ────────────────────────────────────────

    /// Publish a new skill listing.
    pub async fn publish_listing(&self, listing: &SkillListingRow) -> bool {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        match db.execute(
            "INSERT OR REPLACE INTO skill_listings (id, name, description, publisher_id, capabilities, tags, price, version, rating, rating_count, downloads, active, source, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                listing.id, listing.name, listing.description, listing.publisher_id,
                listing.capabilities_json, listing.tags_json,
                listing.price, listing.version,
                listing.rating, listing.rating_count, listing.downloads,
                listing.active as i32, listing.source, listing.metadata_json,
                now, now,
            ],
        ) {
            Ok(_) => true,
            Err(e) => { warn!("Failed to publish listing {}: {}", listing.id, e); false }
        }
    }

    /// Get a listing by ID.
    pub async fn get_listing(&self, id: &str) -> Option<SkillListingRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, name, description, publisher_id, capabilities, tags, price, version, rating, rating_count, downloads, active, source, metadata, created_at, updated_at
             FROM skill_listings WHERE id = ?1",
            params![id],
            |row| Ok(row_to_listing(row)),
        ).ok()
    }

    /// List all active listings.
    pub async fn list_active_listings(&self) -> Vec<SkillListingRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, name, description, publisher_id, capabilities, tags, price, version, rating, rating_count, downloads, active, source, metadata, created_at, updated_at
             FROM skill_listings WHERE active = 1 ORDER BY downloads DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map([], |row| Ok(row_to_listing(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Search listings by name, description, tags, or capabilities.
    pub async fn search_listings(&self, query: &str) -> Vec<SkillListingRow> {
        let db = self.db.lock().await;
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = match db.prepare(
            "SELECT id, name, description, publisher_id, capabilities, tags, price, version, rating, rating_count, downloads, active, source, metadata, created_at, updated_at
             FROM skill_listings
             WHERE active = 1 AND (LOWER(name) LIKE ?1 OR LOWER(description) LIKE ?1 OR LOWER(tags) LIKE ?1 OR LOWER(capabilities) LIKE ?1)
             ORDER BY downloads DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![pattern], |row| Ok(row_to_listing(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// List featured/top listings (by downloads + rating).
    pub async fn featured_listings(&self, limit: u32) -> Vec<SkillListingRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, name, description, publisher_id, capabilities, tags, price, version, rating, rating_count, downloads, active, source, metadata, created_at, updated_at
             FROM skill_listings WHERE active = 1
             ORDER BY (rating * rating_count + downloads) DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![limit], |row| Ok(row_to_listing(row))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// Increment download count for a listing.
    pub async fn record_download(&self, id: &str) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "UPDATE skill_listings SET downloads = downloads + 1, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        );
    }

    /// Unpublish (soft-delete) a listing.
    pub async fn unpublish_listing(&self, id: &str) -> bool {
        let db = self.db.lock().await;
        match db.execute(
            "UPDATE skill_listings SET active = 0, updated_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id],
        ) {
            Ok(n) => n > 0,
            Err(_) => false,
        }
    }

    /// Get listing categories (distinct tags across all active listings).
    pub async fn list_categories(&self) -> Vec<CategoryCount> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare("SELECT tags FROM skill_listings WHERE active = 1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for tags_json in rows.flatten() {
                if let Ok(tags) = serde_json::from_str::<Vec<String>>(&tags_json) {
                    for tag in tags {
                        *counts.entry(tag).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut result: Vec<CategoryCount> = counts
            .into_iter()
            .map(|(name, count)| CategoryCount { name, count })
            .collect();
        result.sort_by(|a, b| b.count.cmp(&a.count));
        result
    }

    // ── Token Ledger ──────────────────────────────────────────

    /// Get or create agent balance.
    pub async fn get_balance(&self, agent_id: &str) -> i64 {
        let db = self.db.lock().await;
        match db.query_row(
            "SELECT balance FROM token_balances WHERE agent_id = ?1",
            params![agent_id],
            |row| row.get(0),
        ) {
            Ok(b) => b,
            Err(_) => {
                let _ = db.execute(
                    "INSERT OR IGNORE INTO token_balances (agent_id, balance, total_earned, total_spent) VALUES (?1, 0, 0, 0)",
                    params![agent_id],
                );
                0
            }
        }
    }

    /// Credit tokens to an agent (mint or reward).
    pub async fn credit(&self, agent_id: &str, amount: u64, reason: &str) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "INSERT INTO token_balances (agent_id, balance, total_earned, total_spent)
             VALUES (?1, ?2, ?2, 0)
             ON CONFLICT(agent_id) DO UPDATE SET balance = balance + ?2, total_earned = total_earned + ?2",
            params![agent_id, amount as i64],
        );
        let _ = db.execute(
            "INSERT INTO token_transactions (id, from_agent, to_agent, amount, reason, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            params![
                uuid::Uuid::new_v4().to_string(),
                agent_id,
                amount as i64,
                reason,
                Utc::now().to_rfc3339()
            ],
        );
    }

    /// Transfer tokens between agents. Returns false if insufficient balance.
    pub async fn transfer(&self, from: &str, to: &str, amount: u64, reason: &str) -> bool {
        let db = self.db.lock().await;
        let balance: i64 = db
            .query_row(
                "SELECT balance FROM token_balances WHERE agent_id = ?1",
                params![from],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if balance < amount as i64 {
            return false;
        }

        let _ = db.execute(
            "UPDATE token_balances SET balance = balance - ?1, total_spent = total_spent + ?1 WHERE agent_id = ?2",
            params![amount as i64, from],
        );
        let _ = db.execute(
            "INSERT INTO token_balances (agent_id, balance, total_earned, total_spent)
             VALUES (?1, ?2, ?2, 0)
             ON CONFLICT(agent_id) DO UPDATE SET balance = balance + ?2, total_earned = total_earned + ?2",
            params![to, amount as i64],
        );
        let _ = db.execute(
            "INSERT INTO token_transactions (id, from_agent, to_agent, amount, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                from,
                to,
                amount as i64,
                reason,
                Utc::now().to_rfc3339()
            ],
        );
        true
    }

    /// Get recent transactions for an agent.
    pub async fn agent_transactions(&self, agent_id: &str, limit: u32) -> Vec<TokenTransactionRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, from_agent, to_agent, amount, reason, created_at
             FROM token_transactions WHERE from_agent = ?1 OR to_agent = ?1
             ORDER BY created_at DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![agent_id, limit], |row| {
            Ok(TokenTransactionRow {
                id: row.get(0)?,
                from_agent: row.get(1)?,
                to_agent: row.get(2)?,
                amount: row.get(3)?,
                reason: row.get(4)?,
                created_at: row.get(5)?,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Ratings ───────────────────────────────────────────────

    /// Add a rating/review for a skill.
    pub async fn add_rating(
        &self,
        skill_id: &str,
        reviewer_id: &str,
        reviewer_name: &str,
        score: f64,
        review_text: &str,
    ) {
        let score = score.clamp(0.0, 5.0);
        let db = self.db.lock().await;
        let _ = db.execute(
            "INSERT INTO skill_ratings (id, skill_id, reviewer_id, reviewer_name, score, review_text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                skill_id, reviewer_id, reviewer_name, score, review_text,
                Utc::now().to_rfc3339(),
            ],
        );
        // Update listing's running average
        let _ = db.execute(
            "UPDATE skill_listings SET
                rating = (rating * rating_count + ?1) / (rating_count + 1),
                rating_count = rating_count + 1,
                updated_at = ?2
             WHERE id = ?3",
            params![score, Utc::now().to_rfc3339(), skill_id],
        );
    }

    /// Get ratings for a skill.
    pub async fn get_ratings(&self, skill_id: &str) -> Vec<SkillRatingRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, skill_id, reviewer_id, reviewer_name, score, review_text, created_at
             FROM skill_ratings WHERE skill_id = ?1 ORDER BY created_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        match stmt.query_map(params![skill_id], |row| {
            Ok(SkillRatingRow {
                id: row.get(0)?,
                skill_id: row.get(1)?,
                reviewer_id: row.get(2)?,
                reviewer_name: row.get(3)?,
                score: row.get(4)?,
                review_text: row.get(5)?,
                created_at: row.get(6)?,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    // ── Reputation ────────────────────────────────────────────

    /// Get or create agent reputation.
    pub async fn get_reputation(&self, agent_id: &str) -> AgentReputationRow {
        let db = self.db.lock().await;
        match db.query_row(
            "SELECT agent_id, trust_score, total_trades, successful_trades, failed_trades, avg_skill_rating, skill_rating_count, last_activity
             FROM agent_reputation WHERE agent_id = ?1",
            params![agent_id],
            |row| Ok(AgentReputationRow {
                agent_id: row.get(0)?,
                trust_score: row.get(1)?,
                total_trades: row.get(2)?,
                successful_trades: row.get(3)?,
                failed_trades: row.get(4)?,
                avg_skill_rating: row.get(5)?,
                skill_rating_count: row.get(6)?,
                last_activity: row.get(7)?,
            }),
        ) {
            Ok(r) => r,
            Err(_) => {
                let now = Utc::now().to_rfc3339();
                let _ = db.execute(
                    "INSERT OR IGNORE INTO agent_reputation (agent_id, trust_score, total_trades, successful_trades, failed_trades, avg_skill_rating, skill_rating_count, last_activity)
                     VALUES (?1, 0.5, 0, 0, 0, 0.0, 0, ?2)",
                    params![agent_id, now],
                );
                AgentReputationRow {
                    agent_id: agent_id.to_string(),
                    trust_score: 0.5,
                    total_trades: 0,
                    successful_trades: 0,
                    failed_trades: 0,
                    avg_skill_rating: 0.0,
                    skill_rating_count: 0,
                    last_activity: now,
                }
            }
        }
    }

    /// Record a successful trade for reputation.
    pub async fn record_trade_success(&self, agent_id: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let _ = db.execute(
            "INSERT INTO agent_reputation (agent_id, trust_score, total_trades, successful_trades, failed_trades, avg_skill_rating, skill_rating_count, last_activity)
             VALUES (?1, 0.6, 1, 1, 0, 0.0, 0, ?2)
             ON CONFLICT(agent_id) DO UPDATE SET
                total_trades = total_trades + 1,
                successful_trades = successful_trades + 1,
                trust_score = CAST(successful_trades + 1 AS REAL) / (total_trades + 1),
                last_activity = ?2",
            params![agent_id, now],
        );
    }

    /// Record a failed trade for reputation.
    pub async fn record_trade_failure(&self, agent_id: &str) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let _ = db.execute(
            "INSERT INTO agent_reputation (agent_id, trust_score, total_trades, successful_trades, failed_trades, avg_skill_rating, skill_rating_count, last_activity)
             VALUES (?1, 0.0, 1, 0, 1, 0.0, 0, ?2)
             ON CONFLICT(agent_id) DO UPDATE SET
                total_trades = total_trades + 1,
                failed_trades = failed_trades + 1,
                trust_score = CAST(successful_trades AS REAL) / (total_trades + 1),
                last_activity = ?2",
            params![agent_id, now],
        );
    }

    // ── Bounties ──────────────────────────────────────────────

    /// Post a new bounty. Escrows reward_credits from the poster's balance.
    pub async fn post_bounty(&self, bounty: &BountyRow) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();

        // Check balance
        let balance: i64 = db
            .query_row(
                "SELECT COALESCE((SELECT balance FROM token_balances WHERE agent_id = ?1), 0)",
                params![bounty.poster_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if balance < bounty.reward_credits as i64 {
            return Err(format!(
                "Insufficient balance: have {}, need {}",
                balance, bounty.reward_credits
            ));
        }

        // Deduct from poster (escrow)
        db.execute(
            "UPDATE token_balances SET balance = balance - ?1, total_spent = total_spent + ?1 WHERE agent_id = ?2",
            params![bounty.reward_credits as i64, bounty.poster_id],
        )
        .map_err(|e| format!("Failed to escrow: {}", e))?;

        // Record escrow transaction
        let txn_id = uuid::Uuid::new_v4().to_string();
        let _ = db.execute(
            "INSERT INTO token_transactions (id, from_agent, to_agent, amount, reason, created_at)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
            params![
                txn_id,
                bounty.poster_id,
                bounty.reward_credits as i64,
                format!("bounty_escrow:{}", bounty.id),
                now
            ],
        );

        // Insert bounty
        match db.execute(
            "INSERT INTO bounties (id, poster_id, title, description, reward_credits, skill_tags, deadline, status, room_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9, ?10)",
            params![
                bounty.id, bounty.poster_id, bounty.title, bounty.description,
                bounty.reward_credits as i64, bounty.skill_tags_json,
                bounty.deadline, bounty.room_id, now, now,
            ],
        ) {
            Ok(_) => Ok(true),
            Err(e) => Err(format!("Failed to post bounty: {}", e)),
        }
    }

    /// Get a bounty by ID.
    pub async fn get_bounty(&self, id: &str) -> Option<BountyRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, poster_id, title, description, reward_credits, skill_tags, deadline, status, claimer_id, claimed_at, completed_at, verifier_id, verified_at, room_id, created_at, updated_at
             FROM bounties WHERE id = ?1",
            params![id],
            |row| Ok(row_to_bounty(row)),
        ).ok()
    }

    /// List bounties by status (default: open).
    pub async fn list_bounties(&self, status: Option<&str>, limit: u32) -> Vec<BountyRow> {
        let db = self.db.lock().await;
        let status_filter = status.unwrap_or("open");
        let mut stmt = match db.prepare(
            "SELECT id, poster_id, title, description, reward_credits, skill_tags, deadline, status, claimer_id, claimed_at, completed_at, verifier_id, verified_at, room_id, created_at, updated_at
             FROM bounties WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map(params![status_filter, limit], |row| Ok(row_to_bounty(row)))
        {
            Ok(rows) => rows,
            Err(_) => return vec![],
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// Claim a bounty (agent starts working on it).
    pub async fn claim_bounty(&self, bounty_id: &str, claimer_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let updated = db
            .execute(
                "UPDATE bounties SET status = 'claimed', claimer_id = ?1, claimed_at = ?2, updated_at = ?3
                 WHERE id = ?4 AND status = 'open'",
                params![claimer_id, now, now, bounty_id],
            )
            .map_err(|e| format!("Failed to claim bounty: {}", e))?;
        Ok(updated > 0)
    }

    /// Submit a bounty for verification.
    pub async fn submit_bounty(&self, bounty_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        let updated = db
            .execute(
                "UPDATE bounties SET status = 'submitted', completed_at = ?1, updated_at = ?2
                 WHERE id = ?3 AND status = 'claimed'",
                params![now, now, bounty_id],
            )
            .map_err(|e| format!("Failed to submit bounty: {}", e))?;
        Ok(updated > 0)
    }

    /// Verify (approve) a completed bounty. Pays the claimer from escrow.
    pub async fn verify_bounty(&self, bounty_id: &str, verifier_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();

        // Get bounty details
        let bounty = db
            .query_row(
                "SELECT claimer_id, reward_credits FROM bounties WHERE id = ?1 AND status = 'submitted'",
                params![bounty_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|e| format!("Bounty not found or not submitted: {}", e))?;
        let (claimer_id, reward) = bounty;

        // Pay claimer
        db.execute(
            "INSERT OR IGNORE INTO token_balances (agent_id, balance, total_earned, total_spent) VALUES (?1, 0, 0, 0)",
            params![claimer_id],
        ).map_err(|e| format!("Failed to init claimer balance: {}", e))?;
        db.execute(
            "UPDATE token_balances SET balance = balance + ?1, total_earned = total_earned + ?1 WHERE agent_id = ?2",
            params![reward, claimer_id],
        ).map_err(|e| format!("Failed to pay claimer: {}", e))?;

        // Record payment transaction
        let txn_id = uuid::Uuid::new_v4().to_string();
        let _ = db.execute(
            "INSERT INTO token_transactions (id, from_agent, to_agent, amount, reason, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                txn_id,
                "escrow",
                claimer_id,
                reward,
                format!("bounty_reward:{}", bounty_id),
                now
            ],
        );

        // Update bounty status
        db.execute(
            "UPDATE bounties SET status = 'verified', verifier_id = ?1, verified_at = ?2, updated_at = ?3
             WHERE id = ?4",
            params![verifier_id, now, now, bounty_id],
        ).map_err(|e| format!("Failed to verify bounty: {}", e))?;

        // Record trade success for claimer reputation
        drop(db);
        self.record_trade_success(&claimer_id).await;

        Ok(true)
    }

    /// Cancel a bounty and refund the poster.
    pub async fn cancel_bounty(&self, bounty_id: &str) -> Result<bool, String> {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();

        let bounty = db
            .query_row(
                "SELECT poster_id, reward_credits FROM bounties WHERE id = ?1 AND status IN ('open', 'claimed')",
                params![bounty_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|e| format!("Bounty not found or not cancellable: {}", e))?;
        let (poster_id, reward) = bounty;

        // Refund poster
        db.execute(
            "UPDATE token_balances SET balance = balance + ?1, total_spent = total_spent - ?1 WHERE agent_id = ?2",
            params![reward, poster_id],
        ).map_err(|e| format!("Failed to refund: {}", e))?;

        let txn_id = uuid::Uuid::new_v4().to_string();
        let _ = db.execute(
            "INSERT INTO token_transactions (id, from_agent, to_agent, amount, reason, created_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            params![
                txn_id,
                poster_id,
                reward,
                format!("bounty_refund:{}", bounty_id),
                now
            ],
        );

        db.execute(
            "UPDATE bounties SET status = 'cancelled', updated_at = ?1 WHERE id = ?2",
            params![now, bounty_id],
        )
        .map_err(|e| format!("Failed to cancel bounty: {}", e))?;

        Ok(true)
    }

    // ── Reputation Badges ────────────────────────────────────

    /// Get reputation with computed badge.
    pub async fn get_reputation_with_badge(&self, agent_id: &str) -> AgentReputationWithBadge {
        let rep = self.get_reputation(agent_id).await;
        let badge = compute_badge(&rep);
        AgentReputationWithBadge {
            agent_id: rep.agent_id,
            trust_score: rep.trust_score,
            total_trades: rep.total_trades,
            successful_trades: rep.successful_trades,
            failed_trades: rep.failed_trades,
            avg_skill_rating: rep.avg_skill_rating,
            skill_rating_count: rep.skill_rating_count,
            last_activity: rep.last_activity,
            badge: badge.to_string(),
            badge_color: badge_color(badge),
        }
    }

    // ── Stats ────────────────────────────────────────────────

    pub async fn stats(&self) -> MarketplaceStats {
        let db = self.db.lock().await;
        let total_listings: u64 = db
            .query_row("SELECT COUNT(*) FROM skill_listings", [], |row| row.get(0))
            .unwrap_or(0);
        let active_listings: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM skill_listings WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let total_trades: u64 = db
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))
            .unwrap_or(0);
        let completed_trades: u64 = db
            .query_row(
                "SELECT COUNT(*) FROM trades WHERE status = 'completed'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let total_agents: u64 = db
            .query_row("SELECT COUNT(*) FROM token_balances", [], |row| row.get(0))
            .unwrap_or(0);
        let total_supply: i64 = db
            .query_row(
                "SELECT COALESCE(SUM(balance), 0) FROM token_balances",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let total_ratings: u64 = db
            .query_row("SELECT COUNT(*) FROM skill_ratings", [], |row| row.get(0))
            .unwrap_or(0);

        MarketplaceStats {
            total_listings,
            active_listings,
            total_trades,
            completed_trades,
            total_agents,
            total_supply,
            total_ratings,
        }
    }

    // ── Agent Teams (Phase 5) ────────────────────────────────────

    /// Save or update an agent team
    pub async fn save_team(&self, team: &AgentTeamRow) {
        let db = self.db.lock().await;
        let now = Utc::now().to_rfc3339();
        if let Err(e) = db.execute(
            "INSERT OR REPLACE INTO agent_teams (id, name, members, split_pct, wallet_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, COALESCE((SELECT created_at FROM agent_teams WHERE id = ?1), ?6), ?6)",
            params![team.id, team.name, team.members_json, team.split_pct_json, team.wallet_id, now],
        ) {
            warn!("Failed to save team: {}", e);
        }
    }

    /// Get a team by ID
    pub async fn get_team(&self, id: &str) -> Option<AgentTeamRow> {
        let db = self.db.lock().await;
        db.query_row(
            "SELECT id, name, members, split_pct, wallet_id, created_at, updated_at FROM agent_teams WHERE id = ?1",
            params![id],
            |row| {
                Ok(AgentTeamRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    members_json: row.get(2)?,
                    split_pct_json: row.get(3)?,
                    wallet_id: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        )
        .ok()
    }

    /// List all teams
    pub async fn list_teams(&self) -> Vec<AgentTeamRow> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, name, members, split_pct, wallet_id, created_at, updated_at FROM agent_teams ORDER BY created_at DESC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| {
            Ok(AgentTeamRow {
                id: row.get(0)?,
                name: row.get(1)?,
                members_json: row.get(2)?,
                split_pct_json: row.get(3)?,
                wallet_id: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }
}

// ============================================================================
// Row types
// ============================================================================

fn row_to_listing(row: &rusqlite::Row) -> SkillListingRow {
    SkillListingRow {
        id: row.get(0).unwrap_or_default(),
        name: row.get(1).unwrap_or_default(),
        description: row.get(2).unwrap_or_default(),
        publisher_id: row.get(3).unwrap_or_default(),
        capabilities_json: row.get(4).unwrap_or_default(),
        tags_json: row.get(5).unwrap_or_default(),
        price: row.get(6).unwrap_or(0),
        version: row.get(7).unwrap_or_default(),
        rating: row.get(8).unwrap_or(0.0),
        rating_count: row.get(9).unwrap_or(0),
        downloads: row.get(10).unwrap_or(0),
        active: row.get::<_, i32>(11).unwrap_or(1) != 0,
        source: row.get(12).unwrap_or_default(),
        metadata_json: row.get(13).unwrap_or_default(),
        created_at: row.get(14).unwrap_or_default(),
        updated_at: row.get(15).unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListingRow {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Serializes as both `publisher_id` and `author_agent_id` for frontend compat
    pub publisher_id: String,
    /// JSON string of capabilities array
    pub capabilities_json: String,
    /// JSON string of tags array
    pub tags_json: String,
    pub price: u64,
    pub version: String,
    pub rating: f64,
    pub rating_count: u64,
    pub downloads: u64,
    pub active: bool,
    pub source: String,
    pub metadata_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Frontend-friendly listing response with normalized field names.
/// Converts from `SkillListingRow` to match zeus-marketplace response format.
#[derive(Debug, Clone, Serialize)]
pub struct SkillListingResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub publisher_id: String,
    pub author_agent_id: String,
    pub capabilities: serde_json::Value,
    pub tags: serde_json::Value,
    pub price: u64,
    pub price_tokens: u64,
    pub version: String,
    pub rating: f64,
    pub rating_count: u64,
    pub downloads: u64,
    pub active: bool,
    pub source: String,
    pub metadata: serde_json::Value,
    pub trust_level: u8,
    pub created_at: String,
    pub updated_at: String,
}

impl From<SkillListingRow> for SkillListingResponse {
    fn from(row: SkillListingRow) -> Self {
        let capabilities = serde_json::from_str(&row.capabilities_json)
            .unwrap_or(serde_json::Value::Array(vec![]));
        let tags = serde_json::from_str(&row.tags_json).unwrap_or(serde_json::Value::Array(vec![]));
        let metadata: serde_json::Value =
            serde_json::from_str(&row.metadata_json).unwrap_or(serde_json::json!({}));
        let trust_level = metadata
            .get("trust_level")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;

        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            author_agent_id: row.publisher_id.clone(),
            publisher_id: row.publisher_id,
            capabilities,
            tags,
            price: row.price,
            price_tokens: row.price,
            version: row.version,
            rating: row.rating,
            rating_count: row.rating_count,
            downloads: row.downloads,
            active: row.active,
            source: row.source,
            metadata,
            trust_level,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenTransactionRow {
    pub id: String,
    pub from_agent: Option<String>,
    pub to_agent: Option<String>,
    pub amount: i64,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRatingRow {
    pub id: String,
    pub skill_id: String,
    pub reviewer_id: String,
    pub reviewer_name: String,
    pub score: f64,
    pub review_text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReputationRow {
    pub agent_id: String,
    pub trust_score: f64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub failed_trades: u64,
    pub avg_skill_rating: f64,
    pub skill_rating_count: u64,
    pub last_activity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryCount {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceStats {
    pub total_listings: u64,
    pub active_listings: u64,
    pub total_trades: u64,
    pub completed_trades: u64,
    pub total_agents: u64,
    pub total_supply: i64,
    pub total_ratings: u64,
}

// ── Bounty Row ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyRow {
    pub id: String,
    pub poster_id: String,
    pub title: String,
    pub description: String,
    pub reward_credits: u64,
    pub skill_tags_json: String,
    pub deadline: Option<String>,
    pub status: String, // open, claimed, submitted, verified, cancelled, expired
    pub claimer_id: Option<String>,
    pub claimed_at: Option<String>,
    pub completed_at: Option<String>,
    pub verifier_id: Option<String>,
    pub verified_at: Option<String>,
    pub room_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Agent Team Row (Phase 5) ─────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeamRow {
    pub id: String,
    pub name: String,
    pub members_json: String,
    pub split_pct_json: String,
    pub wallet_id: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Default for BountyRow {
    fn default() -> Self {
        Self {
            id: String::new(),
            poster_id: String::new(),
            title: String::new(),
            description: String::new(),
            reward_credits: 0,
            skill_tags_json: "[]".to_string(),
            deadline: None,
            status: "open".to_string(),
            claimer_id: None,
            claimed_at: None,
            completed_at: None,
            verifier_id: None,
            verified_at: None,
            room_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

/// Frontend-friendly bounty response with parsed JSON fields.
#[derive(Debug, Clone, Serialize)]
pub struct BountyResponse {
    pub id: String,
    pub poster_id: String,
    pub title: String,
    pub description: String,
    pub reward_credits: u64,
    pub skill_tags: serde_json::Value,
    pub deadline: Option<String>,
    pub status: String,
    pub claimer_id: Option<String>,
    pub claimed_at: Option<String>,
    pub completed_at: Option<String>,
    pub verifier_id: Option<String>,
    pub verified_at: Option<String>,
    pub poster_badge: String,
    pub room_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<BountyRow> for BountyResponse {
    fn from(row: BountyRow) -> Self {
        let skill_tags =
            serde_json::from_str(&row.skill_tags_json).unwrap_or(serde_json::Value::Array(vec![]));
        Self {
            id: row.id,
            poster_id: row.poster_id,
            title: row.title,
            description: row.description,
            reward_credits: row.reward_credits,
            skill_tags,
            deadline: row.deadline,
            status: row.status,
            claimer_id: row.claimer_id,
            claimed_at: row.claimed_at,
            completed_at: row.completed_at,
            verifier_id: row.verifier_id,
            verified_at: row.verified_at,
            poster_badge: String::new(), // filled by handler
            room_id: row.room_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

// ── Reputation with Badge ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReputationWithBadge {
    pub agent_id: String,
    pub trust_score: f64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub failed_trades: u64,
    pub avg_skill_rating: f64,
    pub skill_rating_count: u64,
    pub last_activity: String,
    pub badge: String,
    pub badge_color: String,
}

/// Derive a badge label from reputation data.
pub fn compute_badge(rep: &AgentReputationRow) -> &'static str {
    match (rep.trust_score, rep.total_trades) {
        (s, t) if s >= 0.95 && t >= 50 => "Legendary",
        (s, t) if s >= 0.85 && t >= 20 => "Expert",
        (s, _) if s >= 0.70 => "Trusted",
        (_, t) if t >= 5 => "Active",
        _ => "New",
    }
}

/// Badge color for frontend rendering.
pub fn badge_color(badge: &str) -> String {
    match badge {
        "Legendary" => "#FFD700".to_string(), // gold
        "Expert" => "#A855F7".to_string(),    // purple
        "Trusted" => "#3B82F6".to_string(),   // blue
        "Active" => "#22C55E".to_string(),    // green
        _ => "#9CA3AF".to_string(),           // gray
    }
}

// ── Skill Card (for Pantheon chat messages) ─────────────────

/// A skill card that can be embedded in a Pantheon room message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCard {
    pub skill_id: String,
    pub skill_name: String,
    pub publisher_id: String,
    pub publisher_badge: String,
    pub price_tokens: u64,
    pub rating: f64,
    pub rating_count: u64,
    pub tags: serde_json::Value,
    pub description: String,
    pub can_invoke: bool,
}

// ── Helper: row_to_bounty ───────────────────────────────────

fn row_to_bounty(row: &rusqlite::Row) -> BountyRow {
    BountyRow {
        id: row.get(0).unwrap_or_default(),
        poster_id: row.get(1).unwrap_or_default(),
        title: row.get(2).unwrap_or_default(),
        description: row.get(3).unwrap_or_default(),
        reward_credits: row.get::<_, i64>(4).unwrap_or(0) as u64,
        skill_tags_json: row.get(5).unwrap_or_default(),
        deadline: row.get(6).ok(),
        status: row.get(7).unwrap_or_default(),
        claimer_id: row.get(8).ok(),
        claimed_at: row.get(9).ok(),
        completed_at: row.get(10).ok(),
        verifier_id: row.get(11).ok(),
        verified_at: row.get(12).ok(),
        room_id: row.get(13).ok(),
        created_at: row.get(14).unwrap_or_default(),
        updated_at: row.get(15).unwrap_or_default(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_store() -> MarketplaceStore {
        MarketplaceStore::new(&PathBuf::from(":memory:")).unwrap()
    }

    fn test_listing(id: &str, name: &str) -> SkillListingRow {
        SkillListingRow {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("{} skill", name),
            publisher_id: "agent-1".to_string(),
            capabilities_json: r#"["code_review"]"#.to_string(),
            tags_json: r#"["rust","development"]"#.to_string(),
            price: 10,
            version: "1.0.0".to_string(),
            rating: 0.0,
            rating_count: 0,
            downloads: 0,
            active: true,
            source: "clawhub".to_string(),
            metadata_json: "{}".to_string(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_publish_and_get_listing() {
        let store = make_test_store();
        let listing = test_listing("skill-1", "Code Review");
        assert!(store.publish_listing(&listing).await);

        let fetched = store.get_listing("skill-1").await;
        assert!(fetched.is_some());
        let f = fetched.unwrap();
        assert_eq!(f.name, "Code Review");
        assert_eq!(f.price, 10);
        assert!(f.active);
    }

    #[tokio::test]
    async fn test_list_active_listings() {
        let store = make_test_store();
        store.publish_listing(&test_listing("s1", "Alpha")).await;
        store.publish_listing(&test_listing("s2", "Beta")).await;
        store.publish_listing(&test_listing("s3", "Gamma")).await;

        let active = store.list_active_listings().await;
        assert_eq!(active.len(), 3);

        store.unpublish_listing("s2").await;
        let active = store.list_active_listings().await;
        assert_eq!(active.len(), 2);
    }

    #[tokio::test]
    async fn test_search_listings() {
        let store = make_test_store();
        store
            .publish_listing(&test_listing("s1", "Code Review"))
            .await;
        let mut analysis = test_listing("s2", "Data Analysis");
        analysis.capabilities_json = r#"["analytics"]"#.to_string();
        analysis.tags_json = r#"["data","stats"]"#.to_string();
        store.publish_listing(&analysis).await;
        store
            .publish_listing(&test_listing("s3", "Code Formatting"))
            .await;

        let results = store.search_listings("code").await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_featured_listings() {
        let store = make_test_store();
        let mut l1 = test_listing("s1", "Popular");
        l1.downloads = 100;
        store.publish_listing(&l1).await;

        let mut l2 = test_listing("s2", "New");
        l2.downloads = 5;
        store.publish_listing(&l2).await;

        let featured = store.featured_listings(1).await;
        assert_eq!(featured.len(), 1);
        assert_eq!(featured[0].name, "Popular");
    }

    #[tokio::test]
    async fn test_credit_and_balance() {
        let store = make_test_store();
        assert_eq!(store.get_balance("agent-1").await, 0);

        store.credit("agent-1", 500, "initial grant").await;
        assert_eq!(store.get_balance("agent-1").await, 500);

        store.credit("agent-1", 200, "reward").await;
        assert_eq!(store.get_balance("agent-1").await, 700);
    }

    #[tokio::test]
    async fn test_transfer() {
        let store = make_test_store();
        store.credit("buyer", 100, "grant").await;
        store.credit("seller", 50, "grant").await;

        assert!(
            store
                .transfer("buyer", "seller", 30, "skill purchase")
                .await
        );
        assert_eq!(store.get_balance("buyer").await, 70);
        assert_eq!(store.get_balance("seller").await, 80);

        // Insufficient balance
        assert!(!store.transfer("buyer", "seller", 200, "too much").await);
        assert_eq!(store.get_balance("buyer").await, 70); // unchanged
    }

    #[tokio::test]
    async fn test_add_and_get_rating() {
        let store = make_test_store();
        store
            .publish_listing(&test_listing("s1", "Test Skill"))
            .await;

        store
            .add_rating("s1", "reviewer-1", "Alice", 4.5, "Great skill!")
            .await;
        store
            .add_rating("s1", "reviewer-2", "Bob", 3.5, "Good but could improve")
            .await;

        let ratings = store.get_ratings("s1").await;
        assert_eq!(ratings.len(), 2);
        assert_eq!(ratings[0].reviewer_name, "Bob"); // DESC order

        // Check listing rating updated
        let listing = store.get_listing("s1").await.unwrap();
        assert_eq!(listing.rating_count, 2);
        assert!((listing.rating - 4.0).abs() < 0.01); // avg of 4.5 and 3.5
    }

    #[tokio::test]
    async fn test_reputation() {
        let store = make_test_store();
        let rep = store.get_reputation("agent-1").await;
        assert_eq!(rep.trust_score, 0.5); // default

        store.record_trade_success("agent-1").await;
        store.record_trade_success("agent-1").await;
        store.record_trade_failure("agent-1").await;

        let rep = store.get_reputation("agent-1").await;
        assert_eq!(rep.total_trades, 3);
        assert_eq!(rep.successful_trades, 2);
        assert_eq!(rep.failed_trades, 1);
    }

    #[tokio::test]
    async fn test_stats() {
        let store = make_test_store();
        store.publish_listing(&test_listing("s1", "A")).await;
        store.publish_listing(&test_listing("s2", "B")).await;
        store.credit("agent-1", 1000, "grant").await;

        let stats = store.stats().await;
        assert_eq!(stats.total_listings, 2);
        assert_eq!(stats.active_listings, 2);
        assert_eq!(stats.total_supply, 1000);
    }

    #[tokio::test]
    async fn test_categories() {
        let store = make_test_store();
        let mut l1 = test_listing("s1", "A");
        l1.tags_json = r#"["development","rust"]"#.to_string();
        store.publish_listing(&l1).await;

        let mut l2 = test_listing("s2", "B");
        l2.tags_json = r#"["development","python"]"#.to_string();
        store.publish_listing(&l2).await;

        let cats = store.list_categories().await;
        assert!(cats.iter().any(|c| c.name == "development" && c.count == 2));
        assert!(cats.iter().any(|c| c.name == "rust" && c.count == 1));
    }

    #[tokio::test]
    async fn test_transactions_log() {
        let store = make_test_store();
        store.credit("agent-1", 100, "grant").await;
        store.credit("agent-1", 50, "bonus").await;

        let txns = store.agent_transactions("agent-1", 10).await;
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].reason, "bonus"); // DESC order
    }

    // ── Bounty Tests ──────────────────────────────────

    fn test_bounty(id: &str, poster: &str, reward: u64) -> BountyRow {
        BountyRow {
            id: id.to_string(),
            poster_id: poster.to_string(),
            title: format!("Bounty {}", id),
            description: "Test bounty description".to_string(),
            reward_credits: reward,
            skill_tags_json: r#"["rust","code"]"#.to_string(),
            deadline: None,
            status: "open".to_string(),
            claimer_id: None,
            claimed_at: None,
            completed_at: None,
            verifier_id: None,
            verified_at: None,
            room_id: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_post_bounty() {
        let store = make_test_store();
        store.credit("poster-1", 500, "grant").await;

        let bounty = test_bounty("b1", "poster-1", 100);
        assert!(store.post_bounty(&bounty).await.is_ok());

        // Balance should be deducted (escrow)
        assert_eq!(store.get_balance("poster-1").await, 400);

        let fetched = store.get_bounty("b1").await;
        assert!(fetched.is_some());
        let b = fetched.unwrap();
        assert_eq!(b.title, "Bounty b1");
        assert_eq!(b.status, "open");
    }

    #[tokio::test]
    async fn test_post_bounty_insufficient_balance() {
        let store = make_test_store();
        store.credit("poster-1", 50, "grant").await;

        let bounty = test_bounty("b1", "poster-1", 100);
        assert!(store.post_bounty(&bounty).await.is_err());
    }

    #[tokio::test]
    async fn test_bounty_lifecycle() {
        let store = make_test_store();
        store.credit("poster-1", 1000, "grant").await;

        let bounty = test_bounty("b1", "poster-1", 200);
        store.post_bounty(&bounty).await.unwrap();
        assert_eq!(store.get_balance("poster-1").await, 800);

        // Claim
        assert!(store.claim_bounty("b1", "worker-1").await.unwrap());
        let b = store.get_bounty("b1").await.unwrap();
        assert_eq!(b.status, "claimed");
        assert_eq!(b.claimer_id, Some("worker-1".to_string()));

        // Submit
        assert!(store.submit_bounty("b1").await.unwrap());
        let b = store.get_bounty("b1").await.unwrap();
        assert_eq!(b.status, "submitted");

        // Verify — pays worker
        assert!(store.verify_bounty("b1", "poster-1").await.unwrap());
        let b = store.get_bounty("b1").await.unwrap();
        assert_eq!(b.status, "verified");
        assert_eq!(store.get_balance("worker-1").await, 200);
    }

    #[tokio::test]
    async fn test_bounty_cancel_refund() {
        let store = make_test_store();
        store.credit("poster-1", 500, "grant").await;

        let bounty = test_bounty("b1", "poster-1", 150);
        store.post_bounty(&bounty).await.unwrap();
        assert_eq!(store.get_balance("poster-1").await, 350);

        // Cancel — refund
        assert!(store.cancel_bounty("b1").await.unwrap());
        assert_eq!(store.get_balance("poster-1").await, 500);
        let b = store.get_bounty("b1").await.unwrap();
        assert_eq!(b.status, "cancelled");
    }

    #[tokio::test]
    async fn test_list_bounties() {
        let store = make_test_store();
        store.credit("poster-1", 1000, "grant").await;

        store
            .post_bounty(&test_bounty("b1", "poster-1", 100))
            .await
            .unwrap();
        store
            .post_bounty(&test_bounty("b2", "poster-1", 200))
            .await
            .unwrap();
        store
            .post_bounty(&test_bounty("b3", "poster-1", 50))
            .await
            .unwrap();

        let open = store.list_bounties(Some("open"), 10).await;
        assert_eq!(open.len(), 3);

        // Claim one — should not appear in open list
        store.claim_bounty("b1", "worker-1").await.unwrap();
        let open = store.list_bounties(Some("open"), 10).await;
        assert_eq!(open.len(), 2);

        let claimed = store.list_bounties(Some("claimed"), 10).await;
        assert_eq!(claimed.len(), 1);
    }

    // ── Badge Tests ───────────────────────────────────

    #[test]
    fn test_compute_badge() {
        let rep = AgentReputationRow {
            agent_id: "a".to_string(),
            trust_score: 0.96,
            total_trades: 55,
            successful_trades: 53,
            failed_trades: 2,
            avg_skill_rating: 4.8,
            skill_rating_count: 30,
            last_activity: "2026-02-25".to_string(),
        };
        assert_eq!(compute_badge(&rep), "Legendary");

        let rep2 = AgentReputationRow {
            trust_score: 0.86,
            total_trades: 25,
            ..rep.clone()
        };
        assert_eq!(compute_badge(&rep2), "Expert");

        let rep3 = AgentReputationRow {
            trust_score: 0.72,
            total_trades: 3,
            ..rep.clone()
        };
        assert_eq!(compute_badge(&rep3), "Trusted");

        let rep4 = AgentReputationRow {
            trust_score: 0.5,
            total_trades: 8,
            ..rep.clone()
        };
        assert_eq!(compute_badge(&rep4), "Active");

        let rep5 = AgentReputationRow {
            trust_score: 0.3,
            total_trades: 2,
            ..rep.clone()
        };
        assert_eq!(compute_badge(&rep5), "New");
    }

    #[tokio::test]
    async fn test_reputation_with_badge() {
        let store = make_test_store();
        let rep = store.get_reputation_with_badge("new-agent").await;
        assert_eq!(rep.badge, "New");
        assert_eq!(rep.badge_color, "#9CA3AF");
    }
}
