use std::str::FromStr;
use std::time::SystemTime;

use alloy::primitives::Address;
use vm_data::db::Database;
use vm_data::models::{
    project::{NewProject, NewProjectPayload, Project},
    wallet::{NewWallet, StoredWallet},
};
use vm_data::utils::encryption::encrypt;
use vm_data::utils::helpers::f64_to_big_int;
use vm_evm::constants::Network;
use vm_evm::simulation::fetch_pools::get_pools;

use crate::cache;

pub async fn add_project_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    payload: NewProjectPayload,
) -> Result<Project, Box<dyn std::error::Error + Send + Sync>> {
    let network = Network::from_network_name(&payload.network).ok_or("Unsupported network")?;

    let trading_strategy = &payload.trading_strategy;
    if trading_strategy != "VOLUME_MAKER" {
        return Err("Strategy not available".into());
    }

    let pool_str = payload.pool.as_deref().ok_or("Pool is required for EVM projects")?;

    // Fetch pools for the token
    let token_address = Address::from_str(&payload.address)?;
    let token_info = get_pools(db, network.clone(), token_address).await?;

    // Find the user-specified pool in the fetched pool list
    let found_pool = token_info
        .pools
        .iter()
        .find(|p| p.pool == pool_str)
        .ok_or("Pool not found for this token")?;

    // Generate main wallet
    let signer = alloy::signers::local::PrivateKeySigner::random();
    let pk_hex = hex::encode(signer.credential().to_bytes());
    let address = signer.address().to_string();
    let encrypted_pk = encrypt(pk_hex)?;

    let new_project = NewProject {
        user_id,
        address: payload.address.clone(),
        pool: found_pool.pool.clone(),
        pool_type: format!("Uniswap V{}", found_pool.version),
        name: Some(token_info.name.clone()),
        symbol: Some(token_info.symbol.clone()),
        description: None,
        image: None,
        date_added: SystemTime::now(),
        fees: f64_to_big_int(found_pool.fee as f64 / 10000.0),
        trading_strategy: trading_strategy.clone(),
        network: payload.network.clone(),
        decimals: Some(token_info.decimals as i32),
        pair: Some(found_pool.pair.to_string()),
        max_market_impact_bps: Some(10),
        trade_multiplier: Some(2.0),
        slippage: None,
        bundle_enabled: None,
        jito_tip: None,
    };

    // Atomically insert project + main wallet in a single DB transaction
    let new_wallet = NewWallet {
        pk: encrypted_pk,
        address: address.clone(),
        project_id: 0, // placeholder — overwritten inside transaction
        is_main: true,
        date_added: SystemTime::now(),
    };
    let (project, wallet) = db.add_project_with_wallet(&new_project, &new_wallet).await?;

    // Track wallet address
    let _ = cache::track_address(
        project.user_id,
        project.id,
        wallet.address.clone(),
        project.address.clone(),
    )
    .await;

    // Store wallet in cache
    let _ = cache::add_wallet(StoredWallet {
        user_id: project.user_id,
        wallet: wallet.clone(),
    })
    .await;

    Ok(project)
}
