use std::sync::Arc;

use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use solana_sdk::program_pack::Pack;

use crate::models::wallet_info::WalletInfo;

const COMMITMENT: CommitmentConfig = CommitmentConfig {
    commitment: CommitmentLevel::Processed,
};

pub async fn fetch_project_wallets_raw(
    rpc: Arc<RpcClient>,
    token: String,
    wallets: Vec<String>,
    token_program: Pubkey,
) -> Vec<WalletInfo> {
    let n = wallets.len();
    if n == 0 {
        return vec![];
    }

    let token_mint = Pubkey::from_str_const(&token);

    let wallet_pks: Vec<Pubkey> = wallets.iter().map(|w| Pubkey::from_str_const(w)).collect();
    let token_atas: Vec<Pubkey> = wallet_pks.iter().map(|pk| get_associated_token_address_with_program_id(pk, &token_mint, &token_program)).collect();

    // Layout: [wallets..., token_atas..., token_mint]
    let mut all_pubkeys = Vec::with_capacity(2 * n + 1);
    all_pubkeys.extend_from_slice(&wallet_pks);
    all_pubkeys.extend_from_slice(&token_atas);
    all_pubkeys.push(token_mint);

    // Batch fetch all accounts (100 per RPC call)
    let mut all_accounts: Vec<Option<solana_sdk::account::Account>> = Vec::with_capacity(all_pubkeys.len());
    for chunk in all_pubkeys.chunks(100) {
        match rpc.get_multiple_accounts_with_commitment(chunk, COMMITMENT).await {
            Ok(response) => all_accounts.extend(response.value),
            Err(e) => {
                tracing::error!("[FETCH_WALLETS] Batch RPC failed: {}", e);
                all_accounts.extend(std::iter::repeat_n(None, chunk.len()));
            }
        }
    }

    // Parse token mint decimals from the last account
    // Slice to Mint::LEN to support Token-2022 mints with extension data
    let token_decimals = all_accounts.get(2 * n)
        .and_then(|a| a.as_ref())
        .and_then(|a| {
            if a.data.len() >= spl_token::state::Mint::LEN {
                spl_token::state::Mint::unpack(&a.data[..spl_token::state::Mint::LEN]).ok()
            } else {
                None
            }
        })
        .map(|m| m.decimals)
        .unwrap_or(9) as u32;

    let token_divisor = 10u64.pow(token_decimals) as f64;

    // Build results from indexed positions
    (0..n)
        .map(|i| {
            let sol = all_accounts.get(i)
                .and_then(|a| a.as_ref())
                .map(|a| a.lamports as f64 / LAMPORTS_PER_SOL as f64)
                .unwrap_or(0.0);

            // Slice to Account::LEN to support Token-2022 accounts with extension data
            let tokens = all_accounts.get(n + i)
                .and_then(|a| a.as_ref())
                .and_then(|a| {
                    if a.data.len() >= spl_token::state::Account::LEN {
                        spl_token::state::Account::unpack(&a.data[..spl_token::state::Account::LEN]).ok()
                    } else {
                        None
                    }
                })
                .map(|a| a.amount as f64 / token_divisor)
                .unwrap_or(0.0);

            WalletInfo {
                address: wallets[i].clone(),
                tokens,
                sol,
            }
        })
        .collect()
}
