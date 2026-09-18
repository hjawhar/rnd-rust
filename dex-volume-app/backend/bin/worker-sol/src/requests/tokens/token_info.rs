use std::error::Error;

use vm_data::models::streams::PoolInfo;
use vm_solana::markets::generic::{market_pair::GenericPoolInfo, pools::get_pools};

use crate::state::get_state;

pub fn to_pool_info(p: GenericPoolInfo) -> PoolInfo {
    PoolInfo {
        name: p.name,
        pool_address: p.pair.to_string(),
        token0: p.quote_mint,
        token1: p.base_mint,
        fee: p.fees,
        balance0: p.quote_balance,
        balance1: p.base_balance,
    }
}

pub async fn get_token_info_req(
    token: String,
) -> Result<Vec<PoolInfo>, Box<dyn Error + Send + Sync>> {
    let state = get_state();
    let rpc = state.rpc.clone();
    let pools = get_pools(rpc, token.clone()).await?;

    state.set_token_pools(token, pools.clone());

    Ok(pools.into_iter().map(to_pool_info).collect())
}
