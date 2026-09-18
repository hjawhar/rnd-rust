use crate::cache::{
    price_cache::get_sol_price,
    volume_cache::get_daily_volume,
};
use crate::state::get_state;
use vm_data::db::Database;
use vm_data::models::solana::{PortfolioSummary, ProjectWalletInfo, WalletsFinancialsResponse};
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_nats::subjects;
use std::time::Duration;

/// Schedule a debounced push of wallet financials after a trade confirmation.
///
/// Uses Redis SET NX as a per-project lock so that multiple transactions confirming
/// close together (e.g. bundle buy+sell) only trigger a single push. A short delay
/// before reading gives Geyser account updates time to propagate to cache.
pub fn schedule_push_wallets_financials(
    nc: async_nats::Client,
    db: Database,
    project_id: i32,
    user_id: i32,
) {
    tokio::spawn(async move {
        // Debounce: try to acquire a per-project lock (1s TTL).
        // If another tx already scheduled a push for this project, skip.
        let lock_key = format!("sol:push_lock:{}", project_id);
        let mut conn = vm_redis::get_conn().await;
        let acquired: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(1000_u64) // 1s expiry in milliseconds
            .query_async(&mut conn)
            .await
            .unwrap_or(None);

        if acquired.is_none() {
            return;
        }

        // Wait for Geyser account updates to propagate to cache
        tokio::time::sleep(Duration::from_millis(500)).await;

        push_wallets_financials(&nc, &db, project_id, user_id).await;
    });
}

/// Push wallet financials to api-server via NATS pub-sub.
///
/// Reads from Geyser-fed cache (balances + prices) — no RPC calls.
/// Called after the debounce delay to ensure cache has fresh data.
async fn push_wallets_financials(
    nc: &async_nats::Client,
    db: &Database,
    project_id: i32,
    user_id: i32,
) {
    let state = get_state();

    let project = match db.get_project(user_id, project_id).await {
        Ok(Some(p)) => p,
        _ => return,
    };

    let wallets = match db.get_project_wallets(user_id, project_id).await {
        Ok(w) if !w.is_empty() => w,
        _ => return,
    };

    let sol_price = get_sol_price().await.ok().flatten().unwrap_or(0.0);
    let decimals = project.decimals.unwrap_or(6) as u8;
    let token_price = state.get_token_price(project.pool.clone(), decimals).await.unwrap_or(0.0);
    let token_price_usdc = token_price * sol_price;
    let decimals_factor = 10f64.powi(decimals as i32);

    let mut wallet_infos = Vec::with_capacity(wallets.len());
    for w in &wallets {
        let sol_balance = state.get_balance(&w.address).unwrap_or(0.0);
        let raw_token_balance = state.get_token_balance(&project.address, &w.address).unwrap_or(0.0);
        let token_balance = raw_token_balance / decimals_factor;

        let native_usdc = sol_balance * sol_price;
        let token_usdc = token_balance * token_price_usdc;

        wallet_infos.push(ProjectWalletInfo {
            id: w.id,
            main: w.is_main,
            address: w.address.clone(),
            native_balance: sol_balance,
            token_balance,
            native_usdc,
            token_usdc,
            total_usdc: native_usdc + token_usdc,
        });
    }

    let daily_vol = get_daily_volume(project_id)
        .await
        .ok()
        .flatten()
        .unwrap_or(0.0);
    let daily_target =
        vm_data::utils::helpers::big_int_to_f64(project.trading_daily_volume.clone());

    let summary = PortfolioSummary::from_wallets(
        &wallet_infos,
        sol_price,
        token_price_usdc,
        daily_vol,
        daily_target,
    );

    let response = WalletsFinancialsResponse {
        project_id,
        wallets: wallet_infos,
        summary,
    };

    let wrapper = StreamInfo {
        user_id: -1,
        stream_type: StreamType::ResponseWalletsFinancials(response),
    };
    let bytes = serde_json::to_vec(&wrapper).unwrap_or_default();
    let _ = nc
        .publish(subjects::events::sol::WALLETS_FINANCIALS, bytes.into())
        .await;
}
