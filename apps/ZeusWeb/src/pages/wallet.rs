//! Wallet page — on-chain wallet info, balance, address, transactions, and transfer.
//!
//! Mirrors the TUI wallet overlay (`0f0c485c`) using the same `/v1/wallet/onchain/*` API.

use leptos::prelude::*;
use crate::api;

/// On-chain wallet info from `/v1/wallet/onchain`.
#[derive(Clone, Debug, serde::Deserialize, Default)]
pub struct OnchainWalletInfo {
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub sol_lamports: u64,
    #[serde(default)]
    pub sol: f64,
    #[serde(default)]
    pub token_balance: u64,
    #[serde(default)]
    pub token_decimals: u64,
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub cluster: String,
}

/// Transaction signature from `/v1/wallet/onchain/transactions`.
#[derive(Clone, Debug, serde::Deserialize, Default)]
pub struct OnchainTransaction {
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub slot: u64,
    #[serde(default)]
    pub err: Option<serde_json::Value>,
    #[serde(default)]
    pub memo: Option<String>,
    #[serde(default)]
    pub block_time: Option<i64>,
}

/// Transfer preview plan from `/v1/wallet/onchain/transfer/preview`.
#[derive(Clone, Debug, serde::Deserialize, Default)]
pub struct TransferPlan {
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub recipient: String,
    #[serde(default)]
    pub amount: u64,
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub decimals: u8,
    #[serde(default)]
    pub sender_sol_lamports: u64,
    #[serde(default)]
    pub sender_token_balance: u64,
    #[serde(default)]
    pub token_balance_sufficient: bool,
    #[serde(default)]
    pub recipient_ata_exists: bool,
    #[serde(default)]
    pub ata_create_required: bool,
    #[serde(default)]
    pub fee_estimate_lamports: u64,
    #[serde(default)]
    pub balance_after: u64,
    #[serde(default)]
    pub cluster: String,
}

/// Transfer result from `/v1/wallet/onchain/transfer`.
#[derive(Clone, Debug, serde::Deserialize, Default)]
pub struct TransferResult {
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub recipient: String,
    #[serde(default)]
    pub amount: u64,
    #[serde(default)]
    pub mint: String,
    #[serde(default)]
    pub ata_created: bool,
    #[serde(default)]
    pub cluster: String,
}

#[function_component(WalletPage)]
pub fn WalletPage() -> impl IntoView {
    let wallet_info = RwSignal::new(None::<OnchainWalletInfo>);
    let transactions = RwSignal::new(Vec::<OnchainTransaction>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(None::<String>);
    let transfer_recipient = RwSignal::new(String::new());
    let transfer_amount = RwSignal::new(String::new());
    let transfer_plan = RwSignal::new(None::<TransferPlan>);
    let transfer_loading = RwSignal::new(false);
    let transfer_error = RwSignal::new(None::<String>);
    let transfer_success = RwSignal::new(None::<String>);

    // Fetch wallet info on mount
    {
        let wallet_info = wallet_info.clone();
        let transactions = transactions.clone();
        let loading = loading.clone();
        let error = error.clone();
        use_effect_with((), move |_| {
            let wallet_info = wallet_info.clone();
            let transactions = transactions.clone();
            let loading = loading.clone();
            let error = error.clone();
            wasm_bindgen_futures::spawn_local(async move {
                loading.set(true);
                error.set(None);

                // Fetch wallet info
                match api::fetch_json::<OnchainWalletInfo>("/v1/wallet/onchain").await {
                    Ok(info) => {
                        wallet_info.set(Some(info));
                    }
                    Err(e) => {
                        error.set(Some(format!("Failed to load wallet: {}", e)));
                        loading.set(false);
                        return;
                    }
                }

                // Fetch transactions
                match api::fetch_json::<Vec<OnchainTransaction>>("/v1/wallet/onchain/transactions?limit=20").await {
                    Ok(txs) => {
                        transactions.set(txs);
                    }
                    Err(e) => {
                        // Non-fatal — wallet info still shows
                        web_sys::console::warn_1(&format!("Failed to load transactions: {}", e).into());
                    }
                }

                loading.set(false);
            });
            || ()
        });
    }

    // Transfer preflight handler
    let on_preflight = {
        let transfer_recipient = transfer_recipient.clone();
        let transfer_amount = transfer_amount.clone();
        let transfer_plan = transfer_plan.clone();
        let transfer_loading = transfer_loading.clone();
        let transfer_error = transfer_error.clone();
        Callback::from(move |e: SubmitEvent| {
            e.prevent_default();
            let recipient = (*transfer_recipient).clone();
            let amount_str = (*transfer_amount).clone();
            let transfer_plan = transfer_plan.clone();
            let transfer_loading = transfer_loading.clone();
            let transfer_error = transfer_error.clone();

            let amount: u64 = match amount_str.parse() {
                Ok(a) => a,
                Err(_) => {
                    transfer_error.set(Some("Invalid amount".into()));
                    return;
                }
            };

            transfer_loading.set(true);
            transfer_error.set(None);
            transfer_plan.set(None);

            wasm_bindgen_futures::spawn_local(async move {
                match api::post_json::<TransferPlan>("/v1/wallet/onchain/transfer/preview", &serde_json::json!({
                    "recipient": recipient,
                    "amount": amount,
                })).await {
                    Ok(plan) => {
                        transfer_plan.set(Some(plan));
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("402") || msg.contains("insufficient") {
                            transfer_error.set(Some("Insufficient balance".into()));
                        } else if msg.contains("403") || msg.contains("devnet") {
                            transfer_error.set(Some("Transfers are devnet-only".into()));
                        } else {
                            transfer_error.set(Some(format!("Preview failed: {}", msg)));
                        }
                    }
                }
                transfer_loading.set(false);
            });
        })
    };

    // Transfer confirm handler — executes the actual transfer
    let on_confirm = {
        let transfer_plan = transfer_plan.clone();
        let transfer_success = transfer_success.clone();
        let transfer_error = transfer_error.clone();
        let transfer_loading = transfer_loading.clone();
        let transfer_recipient = transfer_recipient.clone();
        let transfer_amount = transfer_amount.clone();
        Callback::from(move |_: MouseEvent| {
            let plan = (*transfer_plan).clone();
            let recipient = (*transfer_recipient).clone();
            let amount_str = (*transfer_amount).clone();
            let transfer_success = transfer_success.clone();
            let transfer_error = transfer_error.clone();
            let transfer_loading = transfer_loading.clone();
            let transfer_plan = transfer_plan.clone();

            if plan.is_some() {
                let amount: u64 = match amount_str.parse() {
                    Ok(a) => a,
                    Err(_) => {
                        transfer_error.set(Some("Invalid amount".into()));
                        return;
                    }
                };

                transfer_loading.set(true);
                transfer_error.set(None);
                transfer_success.set(None);

                wasm_bindgen_futures::spawn_local(async move {
                    match api::post_json::<TransferResult>("/v1/wallet/onchain/transfer", &serde_json::json!({
                        "recipient": recipient,
                        "amount": amount,
                    })).await {
                        Ok(result) => {
                            transfer_success.set(Some(format!("Transfer successful! Signature: {}", result.signature)));
                            transfer_plan.set(None);
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("402") || msg.contains("insufficient") {
                                transfer_error.set(Some("Insufficient balance".into()));
                            } else if msg.contains("403") || msg.contains("devnet") {
                                transfer_error.set(Some("Transfers are devnet-only".into()));
                            } else {
                                transfer_error.set(Some(format!("Transfer failed: {}", msg)));
                            }
                        }
                    }
                    transfer_loading.set(false);
                });
            }
        })
    };

    // Cancel transfer
    let on_cancel = {
        let transfer_plan = transfer_plan.clone();
        let transfer_recipient = transfer_recipient.clone();
        let transfer_amount = transfer_amount.clone();
        let transfer_error = transfer_error.clone();
        Callback::from(move |_: MouseEvent| {
            transfer_plan.set(None);
            transfer_recipient.set(String::new());
            transfer_amount.set(String::new());
            transfer_error.set(None);
        })
    };

    html! {
        <div class="p-6 max-w-4xl mx-auto">
            <h1 class="text-2xl font-bold mb-6 flex items-center gap-2">
                <icon::Wallet />
                {"On-Chain Wallet"}
            </h1>

            if *loading {
                <div class="flex items-center justify-center p-12">
                    <div class="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
                    <span class="ml-3 text-gray-400">{"Loading wallet..."}</span>
                </div>
            } else if let Some(err) = &*error {
                <div class="bg-red-900/20 border border-red-500 rounded-lg p-4 mb-6">
                    <p class="text-red-400">{err}</p>
                </div>
            } else if let Some(info) = &*wallet_info {
                // Wallet info card
                <div class="bg-gray-800 rounded-lg p-6 mb-6">
                    <div class="flex items-center justify-between mb-4">
                        <h2 class="text-lg font-semibold">{"Wallet Info"}</h2>
                        <span class="px-3 py-1 bg-blue-900/30 text-blue-400 rounded-full text-sm">
                            {&info.cluster}
                        </span>
                    </div>

                    <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                        <div>
                            <label class="text-sm text-gray-400">{"Address"}</label>
                            <p class="font-mono text-sm break-all bg-gray-900 p-2 rounded">
                                {&info.address}
                            </p>
                        </div>
                        <div>
                            <label class="text-sm text-gray-400">{"Mint"}</label>
                            <p class="font-mono text-sm break-all bg-gray-900 p-2 rounded">
                                {&info.mint}
                            </p>
                        </div>
                    </div>

                    <div class="grid grid-cols-2 md:grid-cols-4 gap-4 mt-4">
                        <div class="bg-gray-900 p-4 rounded">
                            <label class="text-sm text-gray-400">{"SOL Balance"}</label>
                            <p class="text-2xl font-bold">{format!("{:.4}", info.sol)}</p>
                        </div>
                        <div class="bg-gray-900 p-4 rounded">
                            <label class="text-sm text-gray-400">{"ZEUS Balance"}</label>
                            <p class="text-2xl font-bold">{info.token_balance}</p>
                        </div>
                        <div class="bg-gray-900 p-4 rounded">
                            <label class="text-sm text-gray-400">{"Lamports"}</label>
                            <p class="text-2xl font-bold">{info.sol_lamports}</p>
                        </div>
                        <div class="bg-gray-900 p-4 rounded">
                            <label class="text-sm text-gray-400">{"Decimals"}</label>
                            <p class="text-2xl font-bold">{info.token_decimals}</p>
                        </div>
                    </div>
                </div>

                // Transfer section
                <div class="bg-gray-800 rounded-lg p-6 mb-6">
                    <h2 class="text-lg font-semibold mb-4">{"Transfer ZEUS Tokens"}</h2>

                    if info.cluster != "devnet" {
                        <div class="bg-amber-900/20 border border-amber-500 rounded-lg p-4">
                            <p class="text-amber-400">{"Transfers are only allowed on devnet. Current cluster: "}{&info.cluster}</p>
                        </div>
                    } else if let Some(plan) = &*transfer_plan {
                        // Preview confirmation — plan from /v1/wallet/onchain/transfer/preview
                        <div class="bg-gray-900 rounded-lg p-4 mb-4">
                            <h3 class="font-semibold mb-3">{"Transfer Plan"}</h3>
                            <div class="grid grid-cols-2 gap-2 text-sm">
                                <span class="text-gray-400">{"From:"}</span>
                                <span class="font-mono break-all">{&plan.sender}</span>
                                <span class="text-gray-400">{"To:"}</span>
                                <span class="font-mono break-all">{&plan.recipient}</span>
                                <span class="text-gray-400">{"Amount:"}</span>
                                <span>{plan.amount}{" tokens"}</span>
                                <span class="text-gray-400">{"Fee Estimate:"}</span>
                                <span>{format!("{:.6}", plan.fee_estimate_lamports as f64 / 1_000_000_000.0)}{" SOL"}</span>
                                <span class="text-gray-400">{"Balance After:"}</span>
                                <span>{plan.balance_after}{" tokens"}</span>
                                <span class="text-gray-400">{"Sufficient Balance:"}</span>
                                <span>{if plan.token_balance_sufficient { "✅ Yes" } else { "❌ No" }}</span>
                                <span class="text-gray-400">{"Recipient ATA:"}</span>
                                <span>{if plan.recipient_ata_exists { "Exists" } else { "Will be created" }}</span>
                                <span class="text-gray-400">{"Cluster:"}</span>
                                <span class="capitalize">{&plan.cluster}</span>
                            </div>
                            <div class="flex gap-3 mt-4">
                                <button
                                    onclick={on_confirm}
                                    disabled={*transfer_loading}
                                    class="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:opacity-50 rounded"
                                >
                                    {if *transfer_loading { "Confirming..." } else { "Confirm Transfer" }}
                                </button>
                                <button
                                    onclick={on_cancel}
                                    disabled={*transfer_loading}
                                    class="px-4 py-2 bg-gray-600 hover:bg-gray-700 disabled:opacity-50 rounded"
                                >
                                    {"Cancel"}
                                </button>
                            </div>
                        </div>
                    } else {
                        // Transfer form
                        <form onsubmit={on_preflight}>
                            <div class="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
                                <div>
                                    <label class="block text-sm text-gray-400 mb-1">{"Recipient Address"}</label>
                                    <input
                                        type="text"
                                        value={(*transfer_recipient).clone()}
                                        oninput={let transfer_recipient = transfer_recipient.clone();
                                            Callback::from(move |e: InputEvent| {
                                                if let Some(input) = e.target_dyn_into::<web_sys::HtmlInputElement>() {
                                                    transfer_recipient.set(input.value());
                                                }
                                            })
                                        }
                                        placeholder="Base58 public key"
                                        class="w-full bg-gray-900 border border-gray-700 rounded px-3 py-2 text-white"
                                        required=true
                                    />
                                </div>
                                <div>
                                    <label class="block text-sm text-gray-400 mb-1">{"Amount (ZEUS)"}</label>
                                    <input
                                        type="number"
                                        value={(*transfer_amount).clone()}
                                        oninput={let transfer_amount = transfer_amount.clone();
                                            Callback::from(move |e: InputEvent| {
                                                if let Some(input) = e.target_dyn_into::<web_sys::HtmlInputElement>() {
                                                    transfer_amount.set(input.value());
                                                }
                                            })
                                        }
                                        placeholder="0"
                                        min="1"
                                        class="w-full bg-gray-900 border border-gray-700 rounded px-3 py-2 text-white"
                                        required=true
                                    />
                                </div>
                            </div>
                            if let Some(err) = &*transfer_error {
                                <p class="text-red-400 text-sm mb-3">{err}</p>
                            }
                            if let Some(msg) = &*transfer_success {
                                <p class="text-green-400 text-sm mb-3">{msg}</p>
                            }
                            <button
                                type="submit"
                                disabled={*transfer_loading}
                                class="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 rounded"
                            >
                                {if *transfer_loading { "Building Plan..." } else { "Preview Transfer" }}
                            </button>
                        </form>
                    }
                </div>

                // Transactions
                <div class="bg-gray-800 rounded-lg p-6">
                    <h2 class="text-lg font-semibold mb-4">{"Recent Transactions"}</h2>
                    if transactions.is_empty() {
                        <p class="text-gray-400">{"No recent transactions"}</p>
                    } else {
                        <div class="overflow-x-auto">
                            <table class="w-full">
                                <thead>
                                    <tr class="text-left text-gray-400 text-sm">
                                        <th class="pb-3 pr-4">{"Signature"}</th>
                                        <th class="pb-3 pr-4">{"Slot"}</th>
                                        <th class="pb-3 pr-4">{"Status"}</th>
                                        <th class="pb-3">{"Time"}</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {for transactions.iter().map(|tx| {
                                        let status = if tx.err.is_some() {
                                            html! { <span class="text-red-400">{"Failed"}</span> }
                                        } else {
                                            html! { <span class="text-green-400">{"Success"}</span> }
                                        };
                                        let time = tx.block_time
                                            .map(|t| {
                                                let date = js_sys::Date::new(&(t as f64 * 1000.0).into());
                                                date.to_locale_string("en-US", &serde_json::json!({}).into())
                                            })
                                            .unwrap_or_else(|| "Unknown".into());

                                        html! {
                                            <tr class="border-t border-gray-700">
                                                <td class="py-3 pr-4">
                                                    <a
                                                        href={format!("https://explorer.solana.com/tx/{}?cluster={}", tx.signature, "devnet")}
                                                        target="_blank"
                                                        rel="noopener noreferrer"
                                                        class="text-blue-400 hover:underline font-mono text-sm"
                                                    >
                                                        {api::truncate_str(&tx.signature, 16)}
                                                    </a>
                                                </td>
                                                <td class="py-3 pr-4 text-gray-400">{tx.slot}</td>
                                                <td class="py-3 pr-4">{status}</td>
                                                <td class="py-3 text-gray-400">{time}</td>
                                            </tr>
                                        }
                                    })}
                                </tbody>
                            </table>
                        </div>
                    }
                </div>
            }
        </div>
    }
}
