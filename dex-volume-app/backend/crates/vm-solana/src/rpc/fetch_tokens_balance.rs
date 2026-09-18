use std::{error::Error, sync::Arc};

use solana_account_decoder_client_types::UiAccountData;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::request::TokenAccountsFilter;

pub async fn fetch_tokens_balance(
    rpc: Arc<RpcClient>,
    owner: String,
    mint: String,
) -> Result<f64, Box<dyn Error + Send + Sync>> {
    let wallet_address = Pubkey::from_str_const(&owner);
    let result = rpc
        .get_token_accounts_by_owner_with_commitment(
            &wallet_address,
            TokenAccountsFilter::Mint(Pubkey::from_str_const(&mint)),
            CommitmentConfig {
                commitment: CommitmentLevel::Processed,
            },
        )
        .await;
    if let Ok(result) = result
        && !result.value.is_empty()
            && let UiAccountData::Json(parsed_account) = &result.value[0].account.data
                && let Some(info) = parsed_account.parsed.get("info") {
                    #[allow(non_snake_case)]
                    if let Some(tokenAmount) = info.get("tokenAmount")
                        && let Some(amount) = tokenAmount.get("amount") {
                            let n = amount.as_str().unwrap();
                            return Ok(n.parse::<f64>().unwrap());
                        }
                }
    Ok(0.0)
}
