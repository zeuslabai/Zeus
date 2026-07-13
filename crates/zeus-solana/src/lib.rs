//! Zeus Solana — on-chain SPL token transfers and settlement
//!
//! Provides:
//! - SPL token transfer submission via Solana RPC
//! - Associated Token Account (ATA) creation when needed
//! - `SolanaSettlement` implementing the agora `SettlementProvider` pattern

pub mod settlement;
pub mod transfer;

pub use settlement::SolanaSettlement;
pub use transfer::{
    SolanaTransferError, TransferParams, TransferPlan, TransferResult, build_transfer_plan,
    submit_transfer,
};

// Re-export solana types needed by downstream crates (e.g. zeus-api handlers)
pub use solana_client;
pub use solana_sdk;

use std::str::FromStr;

use solana_client::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};

/// Derive the associated token account (ATA) address for a given wallet + mint.
pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    spl_associated_token_account::get_associated_token_address_with_program_id(
        wallet,
        mint,
        &spl_token_2022::id(),
    )
}

/// Parse a base58 string into a `Pubkey`, returning a friendly error.
pub fn parse_pubkey(s: &str) -> Result<Pubkey, SolanaTransferError> {
    Pubkey::from_str(s).map_err(|e| SolanaTransferError::InvalidAddress(format!("{s}: {e}")))
}

/// Derive the Solana RPC URL from a network name.
///
/// Maps `"solana-devnet"` → devnet, `"solana-mainnet"` → mainnet-beta.
/// Returns `Err` for unrecognized network strings.
pub fn rpc_url_from_network(network: &str) -> Result<String, SolanaTransferError> {
    match network {
        "solana-devnet" => Ok("https://api.devnet.solana.com".to_string()),
        "solana-mainnet" => Ok("https://api.mainnet-beta.solana.com".to_string()),
        _ => Err(SolanaTransferError::InvalidAddress(format!(
            "Unknown Solana network: {network} (expected solana-devnet or solana-mainnet)"
        ))),
    }
}

/// Request an airdrop of SOL from the Solana devnet faucet.
///
/// Returns the transaction signature on success. Only works on devnet —
/// mainnet has no faucet. Typical amount for testing: 1_000_000_000 (1 SOL).
pub fn request_devnet_airdrop(
    rpc_url: &str,
    pubkey: &Pubkey,
    lamports: u64,
) -> Result<String, SolanaTransferError> {
    let client = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
    let sig = client
        .request_airdrop(pubkey, lamports)
        .map_err(|e| SolanaTransferError::Rpc(format!("Airdrop request failed: {e}")))?;

    // Confirm the airdrop transaction so callers can immediately check balance.
    client
        .confirm_transaction(&sig)
        .map_err(|e| SolanaTransferError::Rpc(format!("Airdrop confirmation failed: {e}")))?;

    Ok(sig.to_string())
}

/// Query the SOL balance (in lamports) for a given pubkey.
pub fn get_sol_balance(rpc_url: &str, pubkey: &Pubkey) -> Result<u64, SolanaTransferError> {
    let client = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
    client
        .get_balance(pubkey)
        .map_err(|e| SolanaTransferError::Rpc(format!("Balance query failed: {e}")))
}

/// Query the SPL token balance for an Associated Token Account (ATA).
///
/// Returns the raw token amount (before decimal adjustment). Returns 0 if the
/// ATA does not exist (not yet created).
pub fn get_token_balance(rpc_url: &str, ata: &Pubkey) -> Result<u64, SolanaTransferError> {
    let client = RpcClient::new_with_commitment(rpc_url.to_string(), CommitmentConfig::confirmed());
    get_token_balance_with_client(&client, ata)
}

/// Query the SPL token balance for an ATA using an existing RPC client.
///
/// This is the reusable backend primitive for previewing transfer readiness
/// without creating extra clients or submitting a transaction.
pub fn get_token_balance_with_client(
    client: &RpcClient,
    ata: &Pubkey,
) -> Result<u64, SolanaTransferError> {
    match client.get_token_account_balance(ata) {
        Ok(ui_amount) => parse_raw_token_amount(&ui_amount.amount),
        Err(e) => {
            let msg = e.to_string();
            // AccountNotFound is expected when ATA hasn't been created yet.
            if is_token_account_missing(&msg) {
                Ok(0)
            } else {
                Err(SolanaTransferError::Rpc(format!(
                    "Token balance query failed: {e}"
                )))
            }
        }
    }
}

/// Whether an RPC token-account lookup error means the ATA simply does not exist.
pub fn is_token_account_missing(message: &str) -> bool {
    message.contains("AccountNotFound") || message.contains("could not find account")
}

/// Parse a raw SPL token amount returned by the RPC API.
pub fn parse_raw_token_amount(amount: &str) -> Result<u64, SolanaTransferError> {
    amount.parse::<u64>().map_err(|e| {
        SolanaTransferError::Rpc(format!("Failed to parse token balance '{amount}': {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::{Keypair, Signer};

    #[test]
    fn test_parse_pubkey_valid() {
        // Solana system program — well-known address
        let pk = parse_pubkey("11111111111111111111111111111111").unwrap();
        assert_eq!(pk, Pubkey::default());
    }

    #[test]
    fn test_parse_pubkey_invalid() {
        assert!(parse_pubkey("not-a-pubkey!!!").is_err());
        assert!(parse_pubkey("").is_err());
    }

    #[test]
    fn test_derive_ata_deterministic() {
        let wallet = parse_pubkey("11111111111111111111111111111111").unwrap();
        let mint = parse_pubkey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let ata1 = derive_ata(&wallet, &mint);
        let ata2 = derive_ata(&wallet, &mint);
        assert_eq!(ata1, ata2);
    }

    #[test]
    fn test_rpc_url_from_network_devnet() {
        assert_eq!(
            rpc_url_from_network("solana-devnet").unwrap(),
            "https://api.devnet.solana.com"
        );
    }

    #[test]
    fn test_rpc_url_from_network_mainnet() {
        assert_eq!(
            rpc_url_from_network("solana-mainnet").unwrap(),
            "https://api.mainnet-beta.solana.com"
        );
    }

    #[test]
    fn test_rpc_url_from_network_unknown() {
        assert!(rpc_url_from_network("solana-testnet").is_err());
        assert!(rpc_url_from_network("").is_err());
    }

    #[test]
    fn test_parse_raw_token_amount_valid() {
        assert_eq!(parse_raw_token_amount("0").unwrap(), 0);
        assert_eq!(
            parse_raw_token_amount("18446744073709551615").unwrap(),
            u64::MAX
        );
    }

    #[test]
    fn test_parse_raw_token_amount_invalid() {
        let err = parse_raw_token_amount("not-a-number")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Failed to parse token balance"));
    }

    #[test]
    fn test_is_token_account_missing_matches_expected_rpc_messages() {
        assert!(is_token_account_missing("AccountNotFound: pubkey missing"));
        assert!(is_token_account_missing("could not find account"));
        assert!(!is_token_account_missing("rate limit exceeded"));
    }

    #[test]
    fn test_request_devnet_airdrop_and_balance() {
        if std::env::var("ZEUS_RUN_SOLANA_TESTS").is_err() {
            return;
        }

        let rpc_url = "https://api.devnet.solana.com";
        let kp = Keypair::new();
        let pubkey = kp.pubkey();

        // Airdrop 1 SOL
        let sig = request_devnet_airdrop(rpc_url, &pubkey, 1_000_000_000).unwrap();
        assert!(!sig.is_empty(), "Airdrop signature should not be empty");

        // Check SOL balance — should be >= 1 SOL (minus any fees)
        let balance = get_sol_balance(rpc_url, &pubkey).unwrap();
        assert!(
            balance >= 1_000_000_000,
            "Balance should be >= 1 SOL, got {balance}"
        );

        // Token balance for a non-existent ATA should return 0
        let fake_ata = Pubkey::new_unique();
        let token_bal = get_token_balance(rpc_url, &fake_ata).unwrap();
        assert_eq!(token_bal, 0, "Non-existent ATA should return 0");
    }
}
