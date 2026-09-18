use alloy::primitives::Address;
use vm_data::db::Database;
use vm_data::models::streams::PoolInfo;
use vm_evm::constants::Network;
use vm_evm::helpers::u256_to_f64;
use vm_evm::models::token::CustomToken;
use vm_evm::simulation::fetch_pools::get_pools;
use std::error::Error;
use std::str::FromStr;

pub async fn get_token_info(
    db: &Database,
    network: &Network,
    token: &str,
) -> Result<(CustomToken, Vec<PoolInfo>), Box<dyn Error + Send + Sync>> {
    let token_addr = Address::from_str(token)?;
    let token_info = get_pools(db, network.clone(), token_addr).await?;

    let pools: Vec<PoolInfo> = token_info
        .pools
        .iter()
        .map(|p| PoolInfo {
            name: format!("UNISWAP_V{}", p.version),
            pool_address: p.pool.clone(),
            token1: token_info.token.to_string(),
            token0: p.pair.to_string(),
            fee: p.fee as f64 / 10000.0,
            balance0: u256_to_f64(p.balance),
            balance1: u256_to_f64(p.tokens),
        })
        .collect();

    Ok((token_info, pools))
}
