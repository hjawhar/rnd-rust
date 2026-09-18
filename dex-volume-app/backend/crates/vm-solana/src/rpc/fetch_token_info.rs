use std::{error::Error, str::FromStr, sync::Arc, time::Duration};

use mpl_token_metadata::accounts::Metadata;
use serde_json::Value;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use spl_token::{solana_program::program_pack::Pack, state::Mint};
use spl_token_2022::extension::{BaseStateWithExtensions, StateWithExtensionsOwned};
use spl_token_metadata_interface::state::TokenMetadata;
use tracing::{debug, error};

use crate::models::token_info::TokenInfo;

/// Fetch token metadata with caching and timeout protection
///
/// **Performance optimizations**:
/// - Batches RPC calls (mint + metadata account)
/// - 5s timeout on metadata URI HTTP requests
/// - Cacheable via external cache layer (see worker-sol cache module)
///
/// **Usage**:
/// ```
/// // From worker-sol with cache:
/// let token_info = get_cached_token_info(rpc, mint).await?;
///
/// // Direct (no cache):
/// let token_info = fetch_token_info(rpc, mint).await?;
/// ```
pub async fn fetch_token_info(
    rpc: Arc<RpcClient>,
    mint: String,
) -> Result<TokenInfo, Box<dyn Error + Send + Sync>> {
    let mint_pubkey = Pubkey::from_str(&mint)?;
    let (metadata_pubkey, _) = Metadata::find_pda(&mint_pubkey);

    debug!("[FETCH_TOKEN_INFO] Fetching token info for {}", mint);

    // Batch RPC calls: mint account + metadata account in parallel
    let (mint_result, metadata_result) = tokio::join!(
        rpc.get_account_data(&mint_pubkey),
        rpc.get_account_data(&metadata_pubkey)
    );

    // Parse mint data (slice to Mint::LEN to support Token-2022 mints
    // which have extension data beyond the base 82-byte Mint layout)
    let account_data = mint_result?;
    if account_data.len() < Mint::LEN {
        return Err(format!("Mint account data too short: {} bytes", account_data.len()).into());
    }
    let mint_data = Mint::unpack(&account_data[..Mint::LEN])?;

    let mut mint_asset_metadata = TokenInfo {
        supply: mint_data.supply,
        decimals: mint_data.decimals,
        name: None,
        symbol: None,
        description: None,
        image: None,
    };

    // Parse metadata if available
    if let Ok(metadata_account_data) = metadata_result {
        match Metadata::safe_deserialize(&metadata_account_data.to_vec()) {
            Ok(metadata) => {
                debug!(
                    "[FETCH_TOKEN_INFO] Fetching metadata URI: {}",
                    metadata.uri
                );

                // Fetch metadata URI with 5s timeout (Phase 1.4)
                let reqwest_client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()?;

                match reqwest_client.get(&metadata.uri).send().await {
                    Ok(response) => {
                        match response.text().await {
                            Ok(uri_data) => {
                                if let Ok(response_json) = serde_json::from_str::<Value>(&uri_data) {
                                    if let Some(name) = response_json.get("name") {
                                        mint_asset_metadata.name = name.as_str().map(|s| s.to_string());
                                    }
                                    if let Some(symbol) = response_json.get("symbol") {
                                        mint_asset_metadata.symbol = symbol.as_str().map(|s| s.to_string());
                                    }
                                    if let Some(description) = response_json.get("description") {
                                        mint_asset_metadata.description = description.as_str().map(|s| s.to_string());
                                    }
                                    if let Some(image) = response_json.get("image") {
                                        mint_asset_metadata.image = image.as_str().map(|s| s.to_string());
                                    }
                                    debug!("[FETCH_TOKEN_INFO] Successfully fetched metadata for {}", mint);
                                } else {
                                    error!("[FETCH_TOKEN_INFO] Failed to parse metadata JSON for {}", mint);
                                }
                            }
                            Err(e) => {
                                error!("[FETCH_TOKEN_INFO] Failed to read metadata response for {}: {}", mint, e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("[FETCH_TOKEN_INFO] HTTP request failed for {} (includes timeout errors): {}", mint, e);
                    }
                }
            }
            Err(e) => {
                error!("[FETCH_TOKEN_INFO] Failed to deserialize metadata for {}: {}", mint, e);
            }
        }
    } else {
        debug!("[FETCH_TOKEN_INFO] No Metaplex metadata for {}, trying Token-2022 extension", mint);

        // Fallback: try Token-2022 metadata extension embedded in the mint account
        if let Ok(mint_with_ext) = StateWithExtensionsOwned::<spl_token_2022::state::Mint>::unpack(account_data.clone())
            && let Ok(token_meta) = mint_with_ext.get_variable_len_extension::<TokenMetadata>() {
                debug!("[FETCH_TOKEN_INFO] Found Token-2022 metadata extension for {}", mint);
                if !token_meta.name.is_empty() {
                    mint_asset_metadata.name = Some(token_meta.name.clone());
                }
                if !token_meta.symbol.is_empty() {
                    mint_asset_metadata.symbol = Some(token_meta.symbol.clone());
                }

                // Fetch URI metadata if available (same flow as Metaplex)
                if !token_meta.uri.is_empty() {
                    let reqwest_client = reqwest::Client::builder()
                        .timeout(Duration::from_secs(5))
                        .build()?;

                    match reqwest_client.get(&token_meta.uri).send().await {
                        Ok(response) => {
                            if let Ok(uri_data) = response.text().await
                                && let Ok(response_json) = serde_json::from_str::<Value>(&uri_data) {
                                    if let Some(description) = response_json.get("description") {
                                        mint_asset_metadata.description = description.as_str().map(|s| s.to_string());
                                    }
                                    if let Some(image) = response_json.get("image") {
                                        mint_asset_metadata.image = image.as_str().map(|s| s.to_string());
                                    }
                                    // URI JSON may also have name/symbol — prefer extension values but fill if missing
                                    if mint_asset_metadata.name.is_none()
                                        && let Some(name) = response_json.get("name") {
                                            mint_asset_metadata.name = name.as_str().map(|s| s.to_string());
                                        }
                                    if mint_asset_metadata.symbol.is_none()
                                        && let Some(symbol) = response_json.get("symbol") {
                                            mint_asset_metadata.symbol = symbol.as_str().map(|s| s.to_string());
                                        }
                                    debug!("[FETCH_TOKEN_INFO] Successfully fetched Token-2022 URI metadata for {}", mint);
                                }
                        }
                        Err(e) => {
                            error!("[FETCH_TOKEN_INFO] Token-2022 URI fetch failed for {}: {}", mint, e);
                        }
                    }
                }
            }
    }

    Ok(mint_asset_metadata)
}
