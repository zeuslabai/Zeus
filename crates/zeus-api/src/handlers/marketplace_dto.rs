//! Marketplace HTTP response DTOs (web4 P0-1b cut-3).
//!
//! These are the handler-layer JSON response shapes for the marketplace/economy
//! REST surface. They were previously defined in the `zeus-marketplace` crate,
//! which is being removed in favour of the unified `zeus-agora` engine. The
//! types live here (the handler layer) per the "agora stays a pure engine"
//! split — they are wire contracts, not domain types.
//!
//! **Hard requirement: byte-identical JSON.** Every field name, type, and
//! `serde` rename is preserved exactly as it was in `zeus-marketplace`, so the
//! REST contract (ZeusWeb `fetch_marketplace_*`, external clients) cannot drift.
//! The golden-JSON round-trip tests at the bottom lock this in.
//!
//! NOTE: the `From<...>` conversions that previously lived alongside these
//! structs referenced `zeus-marketplace`'s own domain types (its `SkillListing`
//! / `Transaction`). Those are intentionally NOT ported — the construction sites
//! are rebuilt over agora's types in cut-5 (reroute).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single marketplace listing in a list/detail response.
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

/// A paginated list of marketplace listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListResponse {
    pub listings: Vec<MarketplaceListingResponse>,
    pub total: usize,
}

/// Convert a `marketplace_store` DB row into the legacy wire DTO.
///
/// web4 P0-1c cut-2 (reroute): the `marketplace_list` handler used to build
/// these from `zeus-marketplace`'s `SkillListing`. That construction path now
/// lives over `marketplace_store::SkillListingRow` (the unified store row).
/// **Byte-identical JSON is preserved** — `tags` parses the JSON-string column
/// into `Vec<String>` and `created_at` parses the RFC3339 string into
/// `DateTime<Utc>`, exactly matching the prior contract.
impl From<super::marketplace_store::SkillListingRow> for MarketplaceListingResponse {
    fn from(row: super::marketplace_store::SkillListingRow) -> Self {
        let tags: Vec<String> = serde_json::from_str(&row.tags_json).unwrap_or_default();
        let created_at = row
            .created_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now());
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            author_agent_id: row.publisher_id,
            price_tokens: row.price,
            rating: row.rating,
            downloads: row.downloads,
            tags,
            created_at,
        }
    }
}

/// Result of publishing a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResponse {
    pub skill_id: String,
    pub status: String,
}

/// Result of proposing/executing a trade.
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

/// A single ledger transaction entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResponse {
    pub id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub amount: u64,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
}

/// An agent's ledger view: balance + transaction history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerResponse {
    pub agent_id: String,
    pub balance: u64,
    pub transactions: Vec<TransactionResponse>,
}

/// An agent's reputation summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationResponse {
    pub agent_id: String,
    pub score: f64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub ratings: f64,
    pub trade_success_rate: f64,
}

/// Status of a trade (web4 P0-1c cut-10: ported from the removed
/// `zeus-marketplace` crate — pure data, the lifecycle lives in agora/SQLite).
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

/// A proposed trade between two agents for a skill (web4 P0-1c cut-10: ported
/// from the removed `zeus-marketplace` crate). The store's `propose_trade`
/// consumes this as a pure data carrier; the two constructors below mirror the
/// original `Trade::new` / `Trade::with_message` so the single construction site
/// in `marketplace_handlers` keeps the same ergonomics.
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
            id: uuid::Uuid::new_v4().to_string(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Golden-JSON round-trip tests: lock the wire contract so a field rename or
    // type drift breaks CI rather than silently breaking REST/WebUI clients.

    #[test]
    fn marketplace_listing_response_golden() {
        let ts = "2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let dto = MarketplaceListingResponse {
            id: "id1".into(),
            name: "skill".into(),
            description: "desc".into(),
            author_agent_id: "agent1".into(),
            price_tokens: 42,
            rating: 4.5,
            downloads: 7,
            tags: vec!["a".into(), "b".into()],
            created_at: ts,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({
                "id": "id1",
                "name": "skill",
                "description": "desc",
                "author_agent_id": "agent1",
                "price_tokens": 42,
                "rating": 4.5,
                "downloads": 7,
                "tags": ["a", "b"],
                "created_at": "2024-01-01T00:00:00Z"
            })
        );
        // Round-trip
        let back: MarketplaceListingResponse = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, "id1");
        assert_eq!(back.price_tokens, 42);
    }

    #[test]
    fn marketplace_list_response_golden() {
        let dto = MarketplaceListResponse {
            listings: vec![],
            total: 0,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v, json!({ "listings": [], "total": 0 }));
    }

    #[test]
    fn publish_response_golden() {
        let dto = PublishResponse {
            skill_id: "sk1".into(),
            status: "published".into(),
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v, json!({ "skill_id": "sk1", "status": "published" }));
    }

    #[test]
    fn trade_response_golden() {
        let ts = "2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let dto = TradeResponse {
            trade_id: "t1".into(),
            status: "proposed".into(),
            buyer_id: "b1".into(),
            seller_id: "s1".into(),
            skill_id: "sk1".into(),
            price: 100,
            timestamp: ts,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({
                "trade_id": "t1",
                "status": "proposed",
                "buyer_id": "b1",
                "seller_id": "s1",
                "skill_id": "sk1",
                "price": 100,
                "timestamp": "2024-01-01T00:00:00Z"
            })
        );
    }

    #[test]
    fn transaction_response_golden() {
        let ts = "2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let dto = TransactionResponse {
            id: "tx1".into(),
            from: Some("a".into()),
            to: None,
            amount: 50,
            reason: "trade".into(),
            timestamp: ts,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({
                "id": "tx1",
                "from": "a",
                "to": null,
                "amount": 50,
                "reason": "trade",
                "timestamp": "2024-01-01T00:00:00Z"
            })
        );
    }

    #[test]
    fn ledger_response_golden() {
        let dto = LedgerResponse {
            agent_id: "a1".into(),
            balance: 1000,
            transactions: vec![],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({ "agent_id": "a1", "balance": 1000, "transactions": [] })
        );
    }

    #[test]
    fn reputation_response_golden() {
        let dto = ReputationResponse {
            agent_id: "a1".into(),
            score: 0.9,
            total_trades: 10,
            successful_trades: 9,
            ratings: 4.5,
            trade_success_rate: 0.9,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            v,
            json!({
                "agent_id": "a1",
                "score": 0.9,
                "total_trades": 10,
                "successful_trades": 9,
                "ratings": 4.5,
                "trade_success_rate": 0.9
            })
        );
    }
}
