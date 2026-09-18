use vm_data::db::Database;
use vm_data::models::solana::{PortfolioSummary, ProjectWalletInfo, WalletsFinancialsResponse};
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_nats::subjects;

/// Schedule a debounced push of wallet financials after a trade confirmation.
///
/// Uses Redis SET NX as a per-project lock so that multiple transactions confirming
/// close together (e.g. bundle buy+sell legs) only trigger a single push.
pub fn schedule_push_wallets_financials(
    nc: async_nats::Client,
    db: Database,
    project_id: i32,
    user_id: i32,
    network_name: String,
) {
    tokio::spawn(async move {
        // Debounce: try to acquire a per-project lock (1s TTL).
        // If another tx already scheduled a push for this project, skip.
        let lock_key = format!("evm:push_lock:{}", project_id);
        let mut conn = vm_redis::get_conn().await;
        let acquired: Option<String> = redis::cmd("SET")
            .arg(&lock_key)
            .arg("1")
            .arg("NX")
            .arg("PX")
            .arg(1000_u64)
            .query_async(&mut conn)
            .await
            .unwrap_or(None);

        if acquired.is_none() {
            return;
        }

        // Short delay to let both legs of a bundle confirm
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        push_wallets_financials_inner(&nc, &db, project_id, user_id, &network_name).await;
    });
}

/// Push wallet financials to api-server via NATS pub-sub.
///
/// Fetches ETH + token balances via Multicall3 and derives token price from pool reserves.
/// Called after the debounce delay to ensure both bundle legs have settled.
async fn push_wallets_financials_inner(
    nc: &async_nats::Client,
    db: &Database,
    project_id: i32,
    user_id: i32,
    network_name: &str,
) {
    let network = match vm_evm::constants::Network::from_network_name(network_name) {
        Some(n) => n,
        None => return,
    };

    let project = match db.get_project(user_id, project_id).await {
        Ok(Some(p)) => p,
        _ => return,
    };

    let wallets = match db.get_project_wallets(user_id, project_id).await {
        Ok(w) if !w.is_empty() => w,
        _ => return,
    };

    let addresses: Vec<alloy::primitives::Address> = wallets
        .iter()
        .filter_map(|w| w.address.parse().ok())
        .collect();

    let token_addr: Option<alloy::primitives::Address> = project.address.parse().ok();

    // Fetch ETH + token balances via Multicall3 (~1 RPC call each)
    let (eth_balances, token_balances) = tokio::join!(
        vm_evm::simulation::fetch_balances::get_eth_balances(
            network.clone(),
            addresses.clone(),
        ),
        async {
            if let Some(token) = token_addr {
                vm_evm::simulation::fetch_balances::get_tokens_balances(
                    network.clone(),
                    token,
                    addresses.clone(),
                )
                .await
            } else {
                Ok(vec![
                    alloy::primitives::U256::ZERO;
                    addresses.len()
                ])
            }
        }
    );

    let eth_balances = eth_balances
        .unwrap_or_else(|_| vec![alloy::primitives::U256::ZERO; addresses.len()]);
    let token_balances = token_balances
        .unwrap_or_else(|_| vec![alloy::primitives::U256::ZERO; addresses.len()]);

    let eth_price = crate::cache::get_eth_price()
        .await
        .unwrap_or(None)
        .unwrap_or(0.0);

    let decimals = project.decimals.unwrap_or(18) as u8;
    let decimals_factor = 10f64.powi(decimals as i32);

    // Derive token price from cached pool reserves (set by volume_maker at task start)
    let mut token_price_eth: f64 = 0.0;
    if let Some(pool) = crate::cache::get_pool(project_id).await.unwrap_or(None) {
        if pool.version == 4 {
            // Use cached V4 pool key params (set by volume_maker at startup, avoids DB query per trade)
            if let Some(pkp) = crate::cache::get_v4_pool_key_params(project_id)
                .await
                .unwrap_or(None)
                && let Ok(price) =
                    vm_evm::simulation::market_impact::quote_v4_spot_price(
                        network.clone(),
                        pkp,
                        decimals,
                    )
                    .await
                {
                    token_price_eth = price;
                }
        } else if pool.tokens > alloy::primitives::U256::ZERO {
            token_price_eth = (vm_evm::helpers::u256_to_f64(pool.balance) / 1e18)
                / (vm_evm::helpers::u256_to_f64(pool.tokens) / decimals_factor);
        }
    }

    let token_price_usdc = token_price_eth * eth_price;

    let mut wallet_infos = Vec::with_capacity(wallets.len());
    for (i, w) in wallets.iter().enumerate() {
        let native_bal = vm_evm::helpers::u256_to_f64(
            *eth_balances
                .get(i)
                .unwrap_or(&alloy::primitives::U256::ZERO),
        ) / 1e18;
        let token_bal = vm_evm::helpers::u256_to_f64(
            *token_balances
                .get(i)
                .unwrap_or(&alloy::primitives::U256::ZERO),
        ) / decimals_factor;
        let native_usdc = native_bal * eth_price;
        let token_usdc = token_bal * token_price_usdc;
        wallet_infos.push(ProjectWalletInfo {
            id: w.id,
            main: w.is_main,
            address: w.address.clone(),
            native_balance: native_bal,
            token_balance: token_bal,
            native_usdc,
            token_usdc,
            total_usdc: native_usdc + token_usdc,
        });
    }

    let daily_vol = crate::cache::get_daily_volume(project_id)
        .await
        .unwrap_or(None)
        .unwrap_or(0.0);
    let daily_target =
        vm_data::utils::helpers::big_int_to_f64(project.trading_daily_volume.clone());

    let summary = PortfolioSummary::from_wallets(
        &wallet_infos,
        eth_price,
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
        .publish(subjects::events::evm::WALLETS_FINANCIALS, bytes.into())
        .await;
}
