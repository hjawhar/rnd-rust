use std::{error::Error, str::FromStr, time::SystemTime};

use vm_data::db::Database;
use vm_data::utils::encryption::encrypt;
use vm_data::models::{
    project::{NewProject, Project},
    wallet::{NewWallet, StoredWallet},
};
use solana_keypair::Keypair;
use vm_solana::{
    markets::generic::market_pair::MarketEnum, rpc::fetch_token_info::fetch_token_info,
};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use crate::state::get_state;
use vm_data::utils::helpers::f64_to_big_int;

pub async fn add_project_req(
    db: &Database,
    _nc: &async_nats::Client,
    user_id: i32,
    address: String,
    pool: String,
    trading_strategy: String,
) -> Result<Project, Box<dyn Error + Send + Sync>> {
    let state = get_state();

    tracing::info!("[ADD_PROJECT] user={} address={} pool={} strategy={}", user_id, address, pool, trading_strategy);

    let address_pubkey = Pubkey::from_str(&address)
        .map_err(|e| {
            tracing::error!("[ADD_PROJECT] Invalid address pubkey '{}': {}", address, e);
            format!("Invalid address: {}", e)
        })?;
    let pool_pubkey = Pubkey::from_str(&pool)
        .map_err(|e| {
            tracing::error!("[ADD_PROJECT] Invalid pool pubkey '{}': {}", pool, e);
            format!("Invalid pool: {}", e)
        })?;

    tracing::info!("[ADD_PROJECT] Looking up pool type in cache...");
    let pool_type = match state.get_pool_type(
        &address_pubkey.to_string(),
        &pool_pubkey.to_string(),
    ) {
        Some(pool_type) => {
            tracing::info!("[ADD_PROJECT] Pool type found: {}", pool_type);
            pool_type
        }
        None => {
            tracing::error!("[ADD_PROJECT] Pool not found in cache for token={} pool={}", address, pool);
            return Err("Pool not found".into());
        }
    };

    tracing::info!("[ADD_PROJECT] Fetching token info from RPC...");
    let mint_asset_metadata = match fetch_token_info(state.rpc.clone(), address_pubkey.clone().to_string()).await {
        Ok(meta) => {
            tracing::info!("[ADD_PROJECT] Token info fetched: name={:?} decimals={}", meta.name, meta.decimals);
            meta
        }
        Err(e) => {
            tracing::error!("[ADD_PROJECT] Failed to fetch token info: {}", e);
            return Err(format!("Failed to fetch token info: {}", e).into());
        }
    };

    tracing::info!("[ADD_PROJECT] Fetching pair data...");
    let pair = match state.fetch_pair(pool_pubkey.to_string().clone()).await {
        Ok(Some(pair)) => {
            tracing::info!("[ADD_PROJECT] Pair fetched: {}", pair.name);
            pair
        }
        Ok(None) => {
            tracing::error!("[ADD_PROJECT] Pair not found for pool={}", pool);
            return Err("Failed to get pair".into());
        }
        Err(e) => {
            tracing::error!("[ADD_PROJECT] Failed to fetch pair: {}", e);
            return Err(format!("Failed to fetch pair: {}", e).into());
        }
    };

    let mut generic_pool = pair.generic().clone();
    if let MarketEnum::RaydiumCLMM(pool) = &pair.market {
        let found = state.get_clmm_config(&pool.amm_config.to_string());
        if let Some(found) = found {
            generic_pool.fees = found.trade_fee_rate as f64 / 10000.0;
        }
    }

    if trading_strategy != "VOLUME_MAKER"
        && trading_strategy != "MARKET_MAKER"
        && trading_strategy != "BUY_SELL"
    {
        tracing::error!("[ADD_PROJECT] Invalid strategy: {}", trading_strategy);
        return Err("Strategy not available".into());
    }

    let mut new_project = NewProject {
        user_id,
        address: address_pubkey.clone().to_string(),
        pool: pool_pubkey.to_string(),
        pool_type,
        name: None,
        symbol: None,
        description: None,
        image: None,
        date_added: SystemTime::now(),
        fees: f64_to_big_int(generic_pool.fees),
        trading_strategy,
        network: "solana".to_string(),
        decimals: Some(mint_asset_metadata.decimals as i32),
        pair: None,
        max_market_impact_bps: None,
        trade_multiplier: None,
        slippage: None,
        bundle_enabled: None,
        jito_tip: None,
    };

    new_project.name = mint_asset_metadata.name;
    new_project.symbol = mint_asset_metadata.symbol;
    new_project.description = mint_asset_metadata.description;
    new_project.image = mint_asset_metadata.image;

    let kp = Keypair::new();
    let hash = encrypt(kp.to_base58_string())?;

    tracing::info!("[ADD_PROJECT] Inserting project + wallet into DB...");
    let new_wallet = NewWallet {
        pk: hash,
        address: kp.pubkey().to_string(),
        project_id: 0, // placeholder — overwritten inside transaction
        is_main: true,
        date_added: SystemTime::now(),
    };
    let (project, wallet) = db.add_project_with_wallet(&new_project, &new_wallet).await
        .map_err(|e| {
            tracing::error!("[ADD_PROJECT] DB insert failed: {}", e);
            e
        })?;

    tracing::info!("[ADD_PROJECT] Project created: id={} name={:?}", project.id, project.name);

    // Track token (pool vaults)
    state.track_token(
        project.user_id,
        project.id,
        project.address.clone(),
        project.pool.clone(),
        pair.generic().base_vault,
        pair.generic().quote_vault,
    );

    // Detect token program for correct ATA derivation (Token vs Token-2022)
    let token_program = state.fetch_token_program(&project.address).await
        .unwrap_or(spl_token::ID);

    // Track wallet address
    state.track_address(
        project.user_id,
        project.id,
        wallet.address.clone(),
        project.address.clone(),
        &token_program,
    );

    // Store wallet in cache
    state.add_wallet(StoredWallet {
        user_id: project.user_id,
        wallet: wallet.clone(),
    });

    // Notify Geyser service to refresh subscription
    state.refresh_geyser_subscription().await;

    tracing::info!("[ADD_PROJECT] Done — project {} fully initialized", project.id);

    Ok(project)
}
