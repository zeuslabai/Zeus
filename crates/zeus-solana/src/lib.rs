//! Zeus Solana — on-chain SPL token transfers and settlement
//!
//! Provides:
//! - SPL token transfer submission via Solana RPC
//! - Associated Token Account (ATA) creation when needed
//! - `SolanaSettlement` implementing the agora `SettlementProvider` pattern

pub mod settlement;
pub mod transfer;

pub use settlement::SolanaSettlement;
pub use transfer::{SolanaTransferError, TransferParams, TransferResult, submit_transfer};

use std::str::FromStr;

use solana_sdk::pubkey::Pubkey;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
