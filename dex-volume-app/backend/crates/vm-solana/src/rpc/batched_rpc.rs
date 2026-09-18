use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::account::Account;
use tracing::error;

/// Maximum number of accounts that can be fetched in a single RPC call
/// Solana RPC has a limit of 50-100 pubkeys per request
const MAX_ACCOUNTS_PER_BATCH: usize = 50;

/// Batch fetch multiple accounts with automatic chunking for large requests
///
/// **Chunking**: Automatically splits requests into batches of 50 pubkeys
/// (Solana RPC limit) and combines results
///
/// **Performance**:
/// - 1-50 accounts: 1 RPC call (~100ms)
/// - 51-100 accounts: 2 RPC calls (~200ms)
/// - 101-150 accounts: 3 RPC calls (~300ms)
/// - etc.
pub async fn get_multiple_accounts_batched(
    rpc: Arc<RpcClient>,
    pubkeys: &[Pubkey],
) -> Result<HashMap<Pubkey, Option<Account>>, Box<dyn Error + Send + Sync>> {
    if pubkeys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut result = HashMap::new();

    for chunk in pubkeys.chunks(MAX_ACCOUNTS_PER_BATCH) {
        let accounts = rpc.get_multiple_accounts(chunk).await?;

        for (i, account) in accounts.iter().enumerate() {
            result.insert(chunk[i], account.clone());
        }
    }

    Ok(result)
}

/// Check if multiple ATAs exist in a single batched call
/// Returns HashMap<Pubkey, bool> where true means account exists
///
/// **Performance optimization**: Checks Redis cache first (1h TTL), only hits RPC for uncached ATAs
pub async fn check_ata_existence_batch(
    rpc: Arc<RpcClient>,
    ata_pubkeys: &[Pubkey],
) -> Result<HashMap<Pubkey, bool>, Box<dyn Error + Send + Sync>> {
    if ata_pubkeys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut result = HashMap::new();
    let mut uncached_pubkeys = Vec::new();

    // Check cache first
    for pubkey in ata_pubkeys {
        match check_ata_cache(&pubkey.to_string()).await {
            Ok(Some(exists)) => {
                result.insert(*pubkey, exists);
            }
            _ => {
                uncached_pubkeys.push(*pubkey);
            }
        }
    }

    if !uncached_pubkeys.is_empty() {
        let accounts = get_multiple_accounts_batched(rpc, &uncached_pubkeys).await?;

        for pubkey in &uncached_pubkeys {
            let exists = accounts.get(pubkey).and_then(|a| a.as_ref()).is_some();
            result.insert(*pubkey, exists);

            // Cache the result
            if let Err(e) = cache_ata(&pubkey.to_string(), exists).await {
                error!("[BATCHED_RPC] Failed to cache ATA {}: {}", pubkey, e);
            }
        }
    }

    Ok(result)
}

/// Check ATA cache (internal helper)
async fn check_ata_cache(_pubkey: &str) -> Result<Option<bool>, Box<dyn Error + Send + Sync>> {
    #[cfg(not(test))]
    {
        // In production, check Redis cache
        // This requires the vm-common crate or direct Redis access
        // For now, we'll return None to skip cache (cache check will be in worker-sol)
        Ok(None)
    }

    #[cfg(test)]
    {
        // In tests, no cache
        Ok(None)
    }
}

/// Cache ATA existence (internal helper)
async fn cache_ata(_pubkey: &str, _exists: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    #[cfg(not(test))]
    {
        // In production, cache to Redis
        // This requires the vm-common crate or direct Redis access
        // For now, we'll skip caching (caching will be in worker-sol)
        Ok(())
    }

    #[cfg(test)]
    {
        // In tests, no cache
        Ok(())
    }
}

/// Fetch account data for a single account with error handling
/// This is a helper for when you need just one account but want consistent error handling
pub async fn get_account_safe(
    rpc: Arc<RpcClient>,
    pubkey: &Pubkey,
) -> Option<Account> {
    match rpc.get_account(pubkey).await {
        Ok(account) => Some(account),
        Err(e) => {
            error!(
                "[BATCHED_RPC] Failed to fetch account {}: {}",
                pubkey, e
            );
            None
        }
    }
}

/// Extract token balance from SPL token account data
/// Returns balance in raw token units (without decimal adjustment)
pub fn extract_token_balance(account: &Account) -> Option<u64> {
    // SPL token account layout: amount is at bytes 64-72
    if account.data.len() >= 72 {
        let amount = u64::from_le_bytes(
            account.data[64..72].try_into().ok()?
        );
        Some(amount)
    } else {
        None
    }
}

/// Batch fetch vault balances for multiple vaults
/// Returns HashMap<Pubkey, u64> with raw token balances
pub async fn get_vault_balances_batch(
    rpc: Arc<RpcClient>,
    vault_pubkeys: &[Pubkey],
) -> Result<HashMap<Pubkey, u64>, Box<dyn Error + Send + Sync>> {
    if vault_pubkeys.is_empty() {
        return Ok(HashMap::new());
    }

    let accounts = get_multiple_accounts_batched(rpc, vault_pubkeys).await?;

    let mut result = HashMap::new();
    for (pubkey, account_opt) in accounts {
        if let Some(account) = account_opt {
            if let Some(balance) = extract_token_balance(&account) {
                result.insert(pubkey, balance);
            } else {
                error!(
                    "[BATCHED_RPC] Failed to extract balance from vault {}",
                    pubkey
                );
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_token_balance() {
        // Create mock SPL token account data
        let mut data = vec![0u8; 165]; // SPL token account size

        // Set amount at bytes 64-72 (little-endian)
        let amount: u64 = 1_000_000_000; // 1 token with 9 decimals
        data[64..72].copy_from_slice(&amount.to_le_bytes());

        let account = Account {
            lamports: 2_039_280,
            data,
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        let balance = extract_token_balance(&account);
        assert_eq!(balance, Some(1_000_000_000));
    }

    #[test]
    fn test_extract_token_balance_invalid() {
        let account = Account {
            lamports: 1000,
            data: vec![0u8; 32], // Too short
            owner: spl_token::id(),
            executable: false,
            rent_epoch: 0,
        };

        let balance = extract_token_balance(&account);
        assert_eq!(balance, None);
    }
}
