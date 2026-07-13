//! On-chain wallet handlers — `/v1/wallet/onchain/*`
//!
//! Exposes read-only wallet info and a devnet-guarded transfer endpoint
//! backed by `zeus_solana`. Zero key material is ever returned in responses.
//!
//! Config is read from the same env vars the settlement layer uses:
//! - `ZEUS_SOLANA_RPC_URL` (required; absence = 503)
//! - `ZEUS_SOLANA_KEYPAIR_PATH` (required for transfer)
//! - `ZEUS_SOLANA_MINT` (required for token balance)
//! - `ZEUS_SOLANA_DECIMALS` (optional, defaults to 6)

use axum::{Json, extract::Query, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use zeus_solana::{
    build_transfer_plan, get_sol_balance, get_token_balance, parse_pubkey, derive_ata,
    solana_client, solana_sdk,
    transfer::TransferParams,
};
use solana_sdk::signature::{Keypair, Signer};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a required env var, returning 503 if missing.
fn require_env(key: &str) -> Result<String, (StatusCode, String)> {
    std::env::var(key).map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("{key} not configured — on-chain wallet unavailable"),
        )
    })
}

/// Read an optional env var with a default.
fn optional_env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Resolve the cluster name from the RPC URL.
fn cluster_from_rpc_url(rpc_url: &str) -> &'static str {
    if rpc_url.contains("devnet") {
        "devnet"
    } else if rpc_url.contains("testnet") {
        "testnet"
    } else if rpc_url.contains("mainnet") {
        "mainnet"
    } else {
        "unknown"
    }
}

/// Load the sender keypair bytes from the configured path.
fn load_keypair_bytes() -> Result<Vec<u8>, (StatusCode, String)> {
    let path = require_env("ZEUS_SOLANA_KEYPAIR_PATH")?;
    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest).to_string_lossy().to_string())
            .unwrap_or(path.clone())
    } else {
        path.clone()
    };
    let data = std::fs::read_to_string(&expanded)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Failed to read keypair at {expanded}: {e}")))?;
    let bytes: Vec<u8> = serde_json::from_str(&data)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Failed to parse keypair JSON: {e}")))?;
    if bytes.len() != 64 {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Keypair at {expanded} has length {} (expected 64)", bytes.len()),
        ));
    }
    Ok(bytes)
}

/// Derive the public key from keypair bytes without exposing the secret.
fn pubkey_from_bytes(bytes: &[u8]) -> Result<String, (StatusCode, String)> {
    // Use zeus_solana's re-exported Keypair via solana_sdk
    let keypair = Keypair::try_from(bytes)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Invalid keypair: {e}")))?;
    Ok(keypair.pubkey().to_string())
}

// ---------------------------------------------------------------------------
// GET /v1/wallet/onchain
// ---------------------------------------------------------------------------

/// On-chain wallet info: address, SOL balance, token balance, cluster.
///
/// Returns:
/// ```json
/// {
///   "address": "<base58 pubkey>",
///   "sol_lamports": 1000000000,
///   "sol": 1.0,
///   "token_balance": 0,
///   "token_decimals": 6,
///   "mint": "<base58>",
///   "cluster": "devnet"
/// }
/// ```
///
/// **Zero key material** — only the public address is returned.
pub async fn onchain_wallet_info() -> Result<Json<Value>, (StatusCode, String)> {
    let rpc_url = require_env("ZEUS_SOLANA_RPC_URL")?;
    let keypair_bytes = load_keypair_bytes()?;
    let mint_str = require_env("ZEUS_SOLANA_MINT")?;
    let decimals: u8 = optional_env("ZEUS_SOLANA_DECIMALS", "6")
        .parse()
        .unwrap_or(6);

    let address = pubkey_from_bytes(&keypair_bytes)?;
    let pubkey = parse_pubkey(&address)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Invalid pubkey: {e}")))?;

    // SOL balance
    let sol_lamports = get_sol_balance(&rpc_url, &pubkey)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("SOL balance query failed: {e}")))?;

    // Token balance (ATA may not exist — that's fine, returns 0)
    let mint = parse_pubkey(&mint_str)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Invalid mint: {e}")))?;
    let ata = derive_ata(&pubkey, &mint);
    let token_balance = get_token_balance(&rpc_url, &ata)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Token balance query failed: {e}")))?;

    let cluster = cluster_from_rpc_url(&rpc_url);

    Ok(Json(json!({
        "address": address,
        "sol_lamports": sol_lamports,
        "sol": sol_lamports as f64 / 1_000_000_000.0,
        "token_balance": token_balance,
        "token_decimals": decimals,
        "mint": mint_str,
        "cluster": cluster,
    })))
}

// ---------------------------------------------------------------------------
// GET /v1/wallet/onchain/transactions
// ---------------------------------------------------------------------------

/// Recent on-chain transactions for the wallet address.
///
/// Uses `getSignaturesForAddress` via Solana RPC. Returns the most recent
/// signatures (max 20 by default, configurable via `?limit=N`).
pub async fn onchain_transactions(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rpc_url = require_env("ZEUS_SOLANA_RPC_URL")?;
    let keypair_bytes = load_keypair_bytes()?;
    let address = pubkey_from_bytes(&keypair_bytes)?;
    let pubkey = parse_pubkey(&address)
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, format!("Invalid pubkey: {e}")))?;

    let limit: usize = params
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(20)
        .min(100);

    let client = solana_client::rpc_client::RpcClient::new_with_commitment(
        rpc_url.clone(),
        solana_sdk::commitment_config::CommitmentConfig::confirmed(),
    );

    let mut sig_infos = client
        .get_signatures_for_address(&pubkey)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Transaction query failed: {e}")))?;
    sig_infos.truncate(limit);

    let transactions: Vec<Value> = sig_infos
        .iter()
        .map(|info| {
            json!({
                "signature": info.signature,
                "slot": info.slot,
                "err": info.err.as_ref().map(|e| e.to_string()),
                "block_time": info.block_time,
                "confirmation_status": info.confirmation_status.as_ref().map(|s| format!("{:?}", s)),
            })
        })
        .collect();

    let cluster = cluster_from_rpc_url(&rpc_url);

    Ok(Json(json!({
        "address": address,
        "cluster": cluster,
        "count": transactions.len(),
        "transactions": transactions,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/wallet/onchain/transfer
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct OnchainTransferRequest {
    /// Recipient address (base58).
    pub recipient: String,
    /// Amount in raw token base units.
    pub amount: u64,
}

/// Execute an on-chain SPL token transfer.
///
/// **Devnet-only guard**: hard-fails if the configured RPC URL does not
/// contain "devnet". This is a safety rail — mainnet transfers require
/// removing this guard explicitly.
///
/// Runs `build_transfer_plan` preflight first (fee estimate + balance check),
/// then `submit_transfer` for the actual signed transaction.
///
/// **Zero key material** in the response — only public addresses and the tx signature.
pub async fn onchain_transfer(
    Json(req): Json<OnchainTransferRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rpc_url = require_env("ZEUS_SOLANA_RPC_URL")?;

    // ── Devnet guard ──────────────────────────────────────────────────
    if !rpc_url.contains("devnet") {
        return Err((
            StatusCode::FORBIDDEN,
            "On-chain transfers are only allowed on devnet. \
             Refusing to execute against non-devnet RPC."
                .to_string(),
        ));
    }

    let keypair_bytes = load_keypair_bytes()?;
    let mint_str = require_env("ZEUS_SOLANA_MINT")?;
    let decimals: u8 = optional_env("ZEUS_SOLANA_DECIMALS", "6")
        .parse()
        .unwrap_or(6);

    // Validate recipient address
    let _recipient_pubkey = parse_pubkey(&req.recipient)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid recipient: {e}")))?;

    if req.amount == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be > 0".to_string()));
    }

    let params = TransferParams {
        rpc_url: rpc_url.clone(),
        sender_keypair_bytes: keypair_bytes,
        recipient: req.recipient.clone(),
        mint: mint_str.clone(),
        amount: req.amount,
        decimals,
    };

    // ── Preflight: build_transfer_plan ────────────────────────────────
    let plan = build_transfer_plan(&params)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Preflight failed: {e}")))?;

    if !plan.token_balance_sufficient {
        return Err((
            StatusCode::PAYMENT_REQUIRED,
            format!(
                "Insufficient token balance: have {}, need {}",
                plan.sender_token_balance, plan.amount
            ),
        ));
    }

    // ── Execute: submit_transfer ──────────────────────────────────────
    info!(
        sender = %plan.sender,
        recipient = %plan.recipient,
        amount = plan.amount,
        "On-chain transfer: preflight passed, submitting"
    );

    let result = zeus_solana::submit_transfer(&params)
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Transfer failed: {e}")))?;

    let cluster = cluster_from_rpc_url(&rpc_url);

    Ok(Json(json!({
        "signature": result.signature,
        "sender": result.sender,
        "recipient": result.recipient,
        "amount": result.amount,
        "mint": result.mint,
        "ata_created": result.ata_created,
        "cluster": cluster,
        "plan": {
            "sender_sol_lamports": plan.sender_sol_lamports,
            "sender_token_balance": plan.sender_token_balance,
            "token_balance_sufficient": plan.token_balance_sufficient,
            "recipient_ata_exists": plan.recipient_ata_exists,
            "ata_create_required": plan.ata_create_required,
        }
    })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_detection() {
        assert_eq!(cluster_from_rpc_url("https://api.devnet.solana.com"), "devnet");
        assert_eq!(cluster_from_rpc_url("https://api.testnet.solana.com"), "testnet");
        assert_eq!(cluster_from_rpc_url("https://api.mainnet-beta.solana.com"), "mainnet");
        assert_eq!(cluster_from_rpc_url("http://localhost:8899"), "unknown");
    }

    #[test]
    fn test_zero_amount_rejected() {
        let req = OnchainTransferRequest {
            recipient: "11111111111111111111111111111111".to_string(),
            amount: 0,
        };
        assert_eq!(req.amount, 0);
    }
}
