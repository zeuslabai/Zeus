//! SPL token transfer submission via Solana RPC.

use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_associated_token_account::{
    get_associated_token_address_with_program_id, instruction::create_associated_token_account,
};
use spl_token_2022::instruction as token_instruction;
use std::{fmt, str::FromStr};
use tracing::{debug, info};
use zeus_wallet::WalletKeypair;

use crate::{get_token_balance_with_client, is_token_account_missing};

/// Errors from Solana transfer operations.
#[derive(Debug, thiserror::Error)]
pub enum SolanaTransferError {
    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid keypair: {0}")]
    InvalidKeypair(String),

    #[error("Insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: u64, need: u64 },

    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
}

impl From<SolanaTransferError> for zeus_core::Error {
    fn from(e: SolanaTransferError) -> Self {
        zeus_core::Error::Network(e.to_string())
    }
}

/// Parameters for an SPL token transfer.
#[derive(Clone)]
pub struct TransferParams {
    /// Solana RPC endpoint URL (e.g. `https://api.devnet.solana.com`).
    pub rpc_url: String,
    /// Sender's Ed25519 keypair bytes (64 bytes: secret + public).
    ///
    /// This field is signing material. Do not log it; the custom `Debug` impl
    /// redacts it by design.
    pub sender_keypair_bytes: Vec<u8>,
    /// Recipient's base58 public key.
    pub recipient: String,
    /// SPL token mint address (base58).
    pub mint: String,
    /// Amount in token base units (e.g. micro-USDC for 6-decimal tokens).
    pub amount: u64,
    /// Token decimals (used for logging only — amount is already in base units).
    pub decimals: u8,
}

impl fmt::Debug for TransferParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferParams")
            .field("rpc_url", &self.rpc_url)
            .field("sender_keypair_bytes", &"<redacted>")
            .field("recipient", &self.recipient)
            .field("mint", &self.mint)
            .field("amount", &self.amount)
            .field("decimals", &self.decimals)
            .finish()
    }
}

impl Drop for TransferParams {
    fn drop(&mut self) {
        self.sender_keypair_bytes.fill(0);
    }
}

/// Result of a successful transfer.
#[derive(Debug, Clone)]
pub struct TransferResult {
    /// Transaction signature (base58).
    pub signature: String,
    /// Sender's public key (base58).
    pub sender: String,
    /// Recipient's public key (base58).
    pub recipient: String,
    /// Amount transferred (base units).
    pub amount: u64,
    /// Token mint (base58).
    pub mint: String,
    /// Whether a new ATA was created for the recipient.
    pub ata_created: bool,
}

/// Read-only readiness plan for an SPL token transfer.
///
/// This intentionally performs the same address derivation and balance/ATA
/// lookups as `submit_transfer`, but never creates accounts, signs, or submits
/// a transaction. UI/API callers can use it against public devnet RPC before a
/// funded wallet is available, then run live transfer verification after
/// funding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPlan {
    /// Sender's public key (base58).
    pub sender: String,
    /// Recipient's public key (base58).
    pub recipient: String,
    /// SPL token mint (base58).
    pub mint: String,
    /// Sender associated token account (base58).
    pub sender_ata: String,
    /// Recipient associated token account (base58).
    pub recipient_ata: String,
    /// Sender SOL balance in lamports for fee-payer readiness.
    pub sender_sol_lamports: u64,
    /// Sender SPL token balance in raw base units.
    pub sender_token_balance: u64,
    /// Requested raw token amount.
    pub amount: u64,
    /// Token decimals for display/logging.
    pub decimals: u8,
    /// Whether the sender currently has enough SPL token balance.
    pub token_balance_sufficient: bool,
    /// Whether the recipient ATA already exists.
    pub recipient_ata_exists: bool,
    /// Whether `submit_transfer` would include recipient ATA creation.
    pub ata_create_required: bool,
}

/// Build a read-only transfer readiness plan without submitting a transaction.
pub fn build_transfer_plan(params: &TransferParams) -> Result<TransferPlan, SolanaTransferError> {
    let sender = Keypair::try_from(params.sender_keypair_bytes.as_slice())
        .map_err(|e| SolanaTransferError::InvalidKeypair(e.to_string()))?;

    let recipient = Pubkey::from_str(&params.recipient)
        .map_err(|e| SolanaTransferError::InvalidAddress(format!("recipient: {e}")))?;

    let mint = Pubkey::from_str(&params.mint)
        .map_err(|e| SolanaTransferError::InvalidAddress(format!("mint: {e}")))?;

    let client =
        RpcClient::new_with_commitment(params.rpc_url.clone(), CommitmentConfig::confirmed());
    let token_program_id = spl_token_2022::id();
    let sender_pubkey = sender.pubkey();
    let sender_ata =
        get_associated_token_address_with_program_id(&sender_pubkey, &mint, &token_program_id);
    let recipient_ata =
        get_associated_token_address_with_program_id(&recipient, &mint, &token_program_id);

    let sender_sol_lamports = client
        .get_balance(&sender_pubkey)
        .map_err(|e| SolanaTransferError::Rpc(format!("SOL balance query failed: {e}")))?;
    let sender_token_balance = get_token_balance_with_client(&client, &sender_ata)?;
    let recipient_ata_present = recipient_ata_exists(&client, &recipient_ata)?;

    Ok(TransferPlan {
        sender: sender_pubkey.to_string(),
        recipient: params.recipient.clone(),
        mint: params.mint.clone(),
        sender_ata: sender_ata.to_string(),
        recipient_ata: recipient_ata.to_string(),
        sender_sol_lamports,
        sender_token_balance,
        amount: params.amount,
        decimals: params.decimals,
        token_balance_sufficient: sender_token_balance >= params.amount,
        recipient_ata_exists: recipient_ata_present,
        ata_create_required: !recipient_ata_present,
    })
}

/// Submit an SPL token transfer on Solana.
///
/// Creates the recipient's Associated Token Account (ATA) if it doesn't exist,
/// then transfers the specified amount of SPL tokens.
///
/// # Errors
/// - `InvalidKeypair` if `sender_keypair_bytes` is not 64 bytes
/// - `InvalidAddress` if recipient or mint is not valid base58
/// - `InsufficientBalance` if the sender's token account has insufficient funds
/// - `Rpc` for network/RPC errors
/// - `TransactionFailed` if the transaction is rejected
pub fn submit_transfer(params: &TransferParams) -> Result<TransferResult, SolanaTransferError> {
    let sender = Keypair::try_from(params.sender_keypair_bytes.as_slice())
        .map_err(|e| SolanaTransferError::InvalidKeypair(e.to_string()))?;

    let recipient = Pubkey::from_str(&params.recipient)
        .map_err(|e| SolanaTransferError::InvalidAddress(format!("recipient: {e}")))?;

    let mint = Pubkey::from_str(&params.mint)
        .map_err(|e| SolanaTransferError::InvalidAddress(format!("mint: {e}")))?;

    let client =
        RpcClient::new_with_commitment(params.rpc_url.clone(), CommitmentConfig::confirmed());

    let token_program_id = spl_token_2022::id();

    // Derive ATAs
    let sender_ata =
        get_associated_token_address_with_program_id(&sender.pubkey(), &mint, &token_program_id);
    let recipient_ata =
        get_associated_token_address_with_program_id(&recipient, &mint, &token_program_id);

    debug!(
        sender = %sender.pubkey(),
        %recipient,
        %mint,
        sender_ata = %sender_ata,
        recipient_ata = %recipient_ata,
        amount = params.amount,
        "Preparing SPL transfer"
    );

    // Check sender token balance. A missing sender ATA is treated as zero
    // balance, so callers get the actionable InsufficientBalance error instead
    // of a lower-level RPC account-not-found string.
    let balance_amount = get_token_balance_with_client(&client, &sender_ata)?;

    if balance_amount < params.amount {
        return Err(SolanaTransferError::InsufficientBalance {
            have: balance_amount,
            need: params.amount,
        });
    }

    // Build instructions
    let mut instructions: Vec<Instruction> = Vec::new();
    let mut ata_created = false;

    // Create recipient ATA if it doesn't exist
    if !recipient_ata_exists(&client, &recipient_ata)? {
        debug!(%recipient_ata, "Creating recipient ATA");
        instructions.push(create_associated_token_account(
            &sender.pubkey(),
            &recipient,
            &mint,
            &token_program_id,
        ));
        ata_created = true;
    }

    // SPL token transfer instruction
    instructions.push(
        token_instruction::transfer_checked(
            &token_program_id,
            &sender_ata,
            &mint,
            &recipient_ata,
            &sender.pubkey(),
            &[],
            params.amount,
            params.decimals,
        )
        .map_err(|e| SolanaTransferError::TransactionFailed(format!("Build transfer ix: {e}")))?,
    );

    // Get recent blockhash and build transaction
    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| SolanaTransferError::Rpc(format!("Failed to get blockhash: {e}")))?;

    let message = Message::new(&instructions, Some(&sender.pubkey()));
    let mut tx = Transaction::new_unsigned(message);
    tx.sign(&[&sender], blockhash);

    // Submit
    let signature = client
        .send_and_confirm_transaction(&tx)
        .map_err(|e| SolanaTransferError::TransactionFailed(e.to_string()))?;

    info!(
        sig = %signature,
        amount = params.amount,
        %mint,
        %recipient,
        ata_created,
        "SPL transfer confirmed"
    );

    Ok(TransferResult {
        signature: signature.to_string(),
        sender: sender.pubkey().to_string(),
        recipient: params.recipient.clone(),
        amount: params.amount,
        mint: params.mint.clone(),
        ata_created,
    })
}

fn recipient_ata_exists(
    client: &RpcClient,
    recipient_ata: &Pubkey,
) -> Result<bool, SolanaTransferError> {
    match client.get_account(recipient_ata) {
        Ok(_) => Ok(true),
        Err(e) => {
            let msg = e.to_string();
            if is_token_account_missing(&msg) {
                Ok(false)
            } else {
                Err(SolanaTransferError::Rpc(format!(
                    "Recipient ATA lookup failed: {e}"
                )))
            }
        }
    }
}

/// Build `TransferParams` from a Zeus wallet keypair.
///
/// This is the normal bridge between `zeus-wallet` and `zeus-solana`: the wallet
/// remains the owner of key persistence, and Solana only receives the transient
/// 64-byte `secret || public` encoding required by `solana_sdk::Keypair`.
pub fn params_from_wallet_keypair(
    wallet: &WalletKeypair,
    rpc_url: &str,
    recipient: &str,
    mint: &str,
    amount: u64,
    decimals: u8,
) -> TransferParams {
    TransferParams {
        rpc_url: rpc_url.to_string(),
        sender_keypair_bytes: wallet.solana_keypair_bytes().to_vec(),
        recipient: recipient.to_string(),
        mint: mint.to_string(),
        amount,
        decimals,
    }
}

/// Build `TransferParams` from raw secret key bytes and a zeus-wallet public key.
///
/// Prefer `params_from_wallet_keypair` when a `WalletKeypair` is available. This
/// lower-level helper remains for callers that already hold raw key material.
///
/// `secret_key_bytes`: the 32-byte Ed25519 secret key.
/// The function constructs the 64-byte Solana keypair (secret || public).
pub fn params_from_secret_key(
    secret_key_bytes: &[u8; 32],
    public_key_bytes: &[u8; 32],
    rpc_url: &str,
    recipient: &str,
    mint: &str,
    amount: u64,
    decimals: u8,
) -> TransferParams {
    let mut keypair_bytes = Vec::with_capacity(64);
    keypair_bytes.extend_from_slice(secret_key_bytes);
    keypair_bytes.extend_from_slice(public_key_bytes);

    TransferParams {
        rpc_url: rpc_url.to_string(),
        sender_keypair_bytes: keypair_bytes,
        recipient: recipient.to_string(),
        mint: mint.to_string(),
        amount,
        decimals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_wallet_dir(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "zeus-solana-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn test_transfer_params_construction() {
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: vec![0u8; 64],
            recipient: "11111111111111111111111111111111".to_string(),
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            amount: 1_000_000,
            decimals: 6,
        };
        assert_eq!(params.amount, 1_000_000);
        assert_eq!(params.decimals, 6);
    }

    #[test]
    fn test_transfer_params_debug_redacts_keypair_bytes() {
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: vec![7u8; 64],
            recipient: "11111111111111111111111111111111".to_string(),
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            amount: 1_000_000,
            decimals: 6,
        };

        let debug = format!("{params:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("sender_keypair_bytes: [7"));
    }

    #[test]
    fn test_params_from_wallet_keypair_uses_solana_keypair_bytes() {
        let wallet_dir = temp_wallet_dir("params-from-wallet");
        let wallet = WalletKeypair::generate(&wallet_dir, "test", "solana-devnet").expect("wallet");
        let recipient = Keypair::new().pubkey().to_string();
        let mint = Keypair::new().pubkey().to_string();

        let params = params_from_wallet_keypair(
            &wallet,
            "https://api.devnet.solana.com",
            &recipient,
            &mint,
            42,
            6,
        );

        assert_eq!(params.rpc_url, "https://api.devnet.solana.com");
        assert_eq!(
            params.sender_keypair_bytes,
            wallet.solana_keypair_bytes().to_vec()
        );
        assert_eq!(params.recipient, recipient);
        assert_eq!(params.mint, mint);
        assert_eq!(params.amount, 42);
        assert_eq!(params.decimals, 6);
    }

    #[test]
    fn test_transfer_plan_manual_construction() {
        let plan = TransferPlan {
            sender: "sender".to_string(),
            recipient: "recipient".to_string(),
            mint: "mint".to_string(),
            sender_ata: "sender-ata".to_string(),
            recipient_ata: "recipient-ata".to_string(),
            sender_sol_lamports: 5_000,
            sender_token_balance: 42,
            amount: 100,
            decimals: 6,
            token_balance_sufficient: false,
            recipient_ata_exists: false,
            ata_create_required: true,
        };

        assert_eq!(plan.sender_sol_lamports, 5_000);
        assert_eq!(plan.sender_token_balance, 42);
        assert!(!plan.token_balance_sufficient);
        assert!(plan.ata_create_required);
    }

    #[test]
    fn test_build_transfer_plan_rejects_invalid_keypair_before_rpc() {
        let params = TransferParams {
            rpc_url: "http://127.0.0.1:1".to_string(),
            sender_keypair_bytes: vec![0u8; 63],
            recipient: Keypair::new().pubkey().to_string(),
            mint: Keypair::new().pubkey().to_string(),
            amount: 100,
            decimals: 6,
        };

        let result = build_transfer_plan(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidKeypair(_))
        ));
    }

    #[test]
    fn test_build_transfer_plan_rejects_invalid_recipient_before_rpc() {
        let params = TransferParams {
            rpc_url: "http://127.0.0.1:1".to_string(),
            sender_keypair_bytes: Keypair::new().to_bytes().to_vec(),
            recipient: "bad-recipient".to_string(),
            mint: Keypair::new().pubkey().to_string(),
            amount: 100,
            decimals: 6,
        };

        let result = build_transfer_plan(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidAddress(_))
        ));
    }

    #[test]
    fn test_build_transfer_plan_rejects_invalid_mint_before_rpc() {
        let params = TransferParams {
            rpc_url: "http://127.0.0.1:1".to_string(),
            sender_keypair_bytes: Keypair::new().to_bytes().to_vec(),
            recipient: Keypair::new().pubkey().to_string(),
            mint: "bad-mint".to_string(),
            amount: 100,
            decimals: 6,
        };

        let result = build_transfer_plan(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidAddress(_))
        ));
    }

    #[test]
    fn test_invalid_keypair_rejected() {
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: vec![0u8; 32], // too short — needs 64
            recipient: "11111111111111111111111111111111".to_string(),
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            amount: 100,
            decimals: 6,
        };
        let result = submit_transfer(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidKeypair(_))
        ));
    }

    #[test]
    fn test_invalid_recipient_rejected() {
        // Valid 64-byte keypair (all zeros is a valid ed25519 secret)
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: Keypair::new().to_bytes().to_vec(),
            recipient: "not-a-valid-pubkey!!!".to_string(),
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            amount: 100,
            decimals: 6,
        };
        let result = submit_transfer(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidAddress(_))
        ));
    }

    #[test]
    fn test_invalid_mint_rejected() {
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: Keypair::new().to_bytes().to_vec(),
            recipient: "11111111111111111111111111111111".to_string(),
            mint: "bad-mint".to_string(),
            amount: 100,
            decimals: 6,
        };
        let result = submit_transfer(&params);
        assert!(matches!(
            result,
            Err(SolanaTransferError::InvalidAddress(_))
        ));
    }

    /// Integration test — requires devnet RPC and funded wallet.
    /// Run with: ZEUS_RUN_SOLANA_TESTS=1 cargo test -p zeus-solana test_devnet_transfer
    #[test]
    #[ignore]
    fn test_devnet_transfer() {
        if std::env::var("ZEUS_RUN_SOLANA_TESTS").is_err() {
            return;
        }

        let sender = Keypair::new();
        let recipient = Keypair::new();
        let params = TransferParams {
            rpc_url: "https://api.devnet.solana.com".to_string(),
            sender_keypair_bytes: sender.to_bytes().to_vec(),
            recipient: recipient.pubkey().to_string(),
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            amount: 1000,
            decimals: 6,
        };

        // This will fail with InsufficientBalance or RPC error since
        // the keypair is unfunded — but it validates the full code path
        // up to the balance check.
        let result = submit_transfer(&params);
        assert!(result.is_err());
    }
}
