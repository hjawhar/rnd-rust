use alloy::primitives::{Address, U256};
use vm_data::db::Database;
use vm_data::models::streams::{DailyVolumeInfo, StreamInfo, StreamType, TaskStatusInfo};
use vm_data::models::task::{EvmProcessBundleBuySell, EvmProcessBuy, EvmProcessSell, EvmProcessTask};
use vm_data::utils::helpers::{big_int_to_f64, modify_by_random_percent};
use vm_evm::constants::Network;
use vm_evm::helpers::{
    f64_to_token_units, f64_to_wei, u256_to_f64, wei_to_f64,
};
use vm_evm::simulation::fetch_eth_price::fetch_eth_price;
use vm_evm::simulation::fetch_pools::get_pools;
use vm_evm::simulation::market_impact::{
    calculate_market_impact_v4, estimate_impact_from_liquidity, quote_v4_spot_price, PoolKeyParams,
};
use vm_nats::subjects;
use rand::Rng;
use rand::SeedableRng;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use chrono::{Timelike, Utc};

use vm_volume::distribution::{cap_trade_to_daily_budget, get_volume_multiplier};

use vm_redis::task_state::MAX_FAILURES;

pub async fn start_volume_maker(
    nc: async_nats::Client,
    db: Database,
    task: EvmProcessTask,
    shutdown: CancellationToken,
    generation: u64,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let project = task.project.clone();
    let project_id = project.id;

    tracing::info!("[EVM] Starting volume maker for project {}", project_id);

    // Store task in cache
    let _ = crate::cache::add_task(task.clone()).await;

    let nc_clone = nc.clone();
    tokio::spawn(async move {
        let network = match Network::from_network_name(&project.network) {
            Some(n) => n,
            None => {
                tracing::error!("[EVM] Unknown network: {}", project.network);
                return;
            }
        };
        let token_address = match Address::from_str(&project.address) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("[EVM] Invalid token address: {}", e);
                return;
            }
        };
        let daily_volume_usdc = big_int_to_f64(project.trading_daily_volume.clone());
        let interval_secs = big_int_to_f64(project.trading_interval.clone()) as u64;

        // Fetch token info once at startup
        let token_info = match get_pools(&db, network.clone(), token_address).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("[EVM] Failed to fetch token pools: {}", e);
                return;
            }
        };

        // Select best pool — find the project's configured pool, or best by (version, liquidity)
        let best_pool = match token_info
            .pools
            .iter()
            .find(|p| p.pool == project.pool)
            .or_else(|| {
                token_info
                    .pools
                    .iter()
                    .max_by_key(|p| (p.version, p.balance))
            }) {
            Some(p) => p.clone(),
            None => {
                tracing::error!("[EVM] No pools found for token");
                return;
            }
        };

        // Cache best pool for push_wallets_financials (avoids pool-fetch RPC on each trade confirm)
        let _ = crate::cache::set_pool(project_id, &best_pool).await;

        let token_info_json = serde_json::to_string(&token_info).unwrap_or_default();
        let pool_info_json = serde_json::to_string(&best_pool).unwrap_or_default();

        // Balance fetching is only supported on Ethereum/Base (BalanceChecker contract)
        let balance_fetch_supported = matches!(network, Network::Ethereum | Network::Base);

        // 15.3: Resolve V4 pool key params once (static metadata, only changes on 60s discovery)
        let v4_pool_key_params: Option<PoolKeyParams> = if best_pool.version == 4 {
            let v4_pools = db
                .get_latest_uniswap_v4_pools_by_address(
                    network.clone() as i32,
                    token_address.to_string(),
                )
                .await
                .unwrap_or_default();
            let matching_pool = v4_pools.iter().find(|p| p.pool_key == best_pool.pool);
            if let Some(v4_pool) = matching_pool {
                Some(PoolKeyParams {
                    currency0: Address::from_str(&v4_pool.currency0).unwrap_or_default(),
                    currency1: Address::from_str(&v4_pool.currency1).unwrap_or_default(),
                    fee: v4_pool.fee.parse::<u32>().unwrap_or(0),
                    tick_spacing: v4_pool.tick_spacing.parse::<i32>().unwrap_or(60),
                    hooks: Address::from_str(&v4_pool.hooks).unwrap_or_default(),
                })
            } else {
                tracing::error!("[EVM] V4 pool key not found in DB for project {}", project_id);
                return;
            }
        } else {
            None
        };

        // Cache V4 pool key params for push_financials (avoids DB query per trade)
        if let Some(ref pkp) = v4_pool_key_params {
            let _ = crate::cache::set_v4_pool_key_params(project_id, pkp).await;
        }

        // 15.4: Parse wallet addresses once (reused every iteration)
        let addresses: Vec<Address> = task
            .wallets
            .iter()
            .filter_map(|w| Address::from_str(&w.address).ok())
            .collect();

        // 15.2: Create RNG once (reused for wallet selection + sleep jitter)
        let mut rng = rand::rngs::StdRng::from_os_rng();

        loop {
            // Refresh task heartbeat (30s TTL) — proves this worker is alive and executing
            let _ = vm_redis::task_heartbeat::refresh_heartbeat("evm", project_id).await;
            // Check for shutdown signal
            if shutdown.is_cancelled() {
                tracing::info!("[EVM] Shutdown signal received, stopping volume maker for project {}", project_id);
                break;
            }

            // Check if task still exists or if superseded by a newer generation
            if let Ok(None) = crate::cache::get_task(project_id).await {
                tracing::info!("[EVM] Task {} removed, stopping volume maker", project_id);
                break;
            }
            match crate::cache::get_task_generation(project_id).await {
                Ok(current_gen) if current_gen != generation => {
                    tracing::info!("[EVM] Project {} | generation {} superseded by {}, exiting", project_id, generation, current_gen);
                    break;
                }
                Ok(_) => {}
                Err(_) => {
                    // Ownership lost entirely (no owner record) — stop
                    tracing::info!("[EVM] Project {} | ownership lost, exiting", project_id);
                    break;
                }
            }

            // Check failure count
            if let Ok(failures) = crate::cache::get_task_failures(project_id).await
                && failures >= MAX_FAILURES {
                    tracing::warn!(
                        "[EVM] Project {} reached {} failures, auto-stopping",
                        project_id,
                        failures
                    );
                    let _ = crate::cache::remove_task(project_id).await;

                    let msg = format!(
                        "Project {} auto-stopped after {} consecutive failures",
                        project_id, failures
                    );
                    let _ = nc_clone
                        .publish(subjects::discord::MESSAGE, msg.into())
                        .await;

                    // Clear heartbeat + release ownership + update DB status
                    let _ = vm_redis::task_heartbeat::clear_heartbeat("evm", project_id).await;
                    let _ = crate::cache::release_task_ownership(project_id).await;
                    let _ = db.update_project_status(project_id, "stopped").await;
                    let status_bytes = serde_json::to_vec(&StreamInfo {
                        user_id: project.user_id,
                        stream_type: StreamType::TaskStatusUpdate(TaskStatusInfo {
                            project_id,
                            status: "stopped".to_string(),
                        }),
                    })
                    .unwrap_or_default();
                    let _ = nc_clone
                        .publish(subjects::events::evm::TASK_STATUS, status_bytes.into())
                        .await;
                    break;
                }

            // Fetch ETH price + daily volume in parallel (independent I/O)
            let (eth_price_res, used_today_res) = tokio::join!(
                fetch_eth_price(&Network::Ethereum),
                crate::cache::get_daily_volume(project_id)
            );

            let eth_price = match eth_price_res {
                Ok(p) => {
                    let _ = crate::cache::set_eth_price(p).await;
                    p
                }
                Err(e) => {
                    tracing::warn!("[EVM] Failed to fetch ETH price: {}", e);
                    let _ = crate::cache::increment_task_failures(project_id).await;
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                }
            };

            if task.wallets.is_empty() {
                tracing::warn!("[EVM] No wallets for project {}", project_id);
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }

            // --- Volume distribution (hourly multiplier + daily budget cap) ---
            let intervals_per_day = 86400.0 / interval_secs as f64;
            let base_value_usdc = daily_volume_usdc / intervals_per_day;

            // Apply organic volume distribution based on hour of day
            let current_hour = Utc::now().hour();
            let volume_multiplier = get_volume_multiplier(current_hour);
            let proposed_value_usdc = base_value_usdc * volume_multiplier;

            // Check daily volume budget and cap if needed
            let used_today = used_today_res
                .unwrap_or(Some(0.0))
                .unwrap_or(0.0);
            let remaining_budget = (daily_volume_usdc - used_today).max(0.0);

            // Minimum trade threshold: $1 or 1% of base value, whichever is larger
            let min_trade_threshold = (base_value_usdc * 0.01).max(1.0);
            let capped =
                cap_trade_to_daily_budget(proposed_value_usdc, remaining_budget, min_trade_threshold);

            if capped.skip_trade {
                tracing::info!(
                    "[EVM] Project {} skipping trade (daily budget exhausted)",
                    project_id
                );
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }

            let value_usdc = capped.value_usdc;
            let mut trade_eth = value_usdc / eth_price;

            // Randomize trade size ±3-8%
            trade_eth = modify_by_random_percent(trade_eth, 3.0, 8.0);

            let mut volume_per_interval_usdc = trade_eth * eth_price;

            // --- Token price ---
            let decimals = token_info.decimals;
            let decimals_factor = 10_f64.powi(decimals as i32);

            let token_price_eth = if let Some(ref pkp) = v4_pool_key_params {
                // V4 CLMM pools have balance=0 in reserves, use on-chain quoter for spot price
                match quote_v4_spot_price(network.clone(), pkp.clone(), decimals).await {
                    Ok(price) => {
                        tracing::debug!("[EVM] Project {} | V4 spot price: {:.10} ETH/token", project_id, price);
                        price
                    }
                    Err(e) => {
                        tracing::warn!("[EVM] Project {} | V4 spot price quote failed: {}", project_id, e);
                        let _ = crate::cache::increment_task_failures(project_id).await;
                        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                        continue;
                    }
                }
            } else if best_pool.tokens > U256::ZERO {
                // V2/V3: derive price from pool reserves
                let price = (u256_to_f64(best_pool.balance) / 1e18)
                    / (u256_to_f64(best_pool.tokens) / decimals_factor);
                tracing::info!("[EVM] Project {} | V2/V3 reserve price: {:.10} ETH/token", project_id, price);
                price
            } else {
                tracing::warn!("[EVM] Project {} | token_price_eth=0 (pool.tokens is ZERO), skipping", project_id);
                let _ = crate::cache::increment_task_failures(project_id).await;
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            };

            // --- Batch balance fetching + portfolio-weighted direction ---
            let (eth_balances, token_balances) = if balance_fetch_supported {
                let (eth_res, tok_res) = tokio::join!(
                    vm_evm::simulation::fetch_balances::get_eth_balances(
                        network.clone(),
                        addresses.clone(),
                    ),
                    vm_evm::simulation::fetch_balances::get_tokens_balances(
                        network.clone(),
                        token_address,
                        addresses.clone(),
                    )
                );
                match (&eth_res, &tok_res) {
                    (Err(e), _) => tracing::warn!("[EVM] Project {} | ETH balance fetch failed: {}", project_id, e),
                    (_, Err(e)) => tracing::warn!("[EVM] Project {} | Token balance fetch failed: {}", project_id, e),
                    _ => {}
                }
                (eth_res.ok(), tok_res.ok())
            } else {
                tracing::warn!("[EVM] Project {} | Balance fetch not supported for network {:?}", project_id, network);
                (None, None)
            };

            // Portfolio ratio — used by both bundle and single-trade modes
            let token_ratio = if let (Some(eth_bals), Some(tok_bals)) =
                (&eth_balances, &token_balances)
            {
                let total_eth: f64 = eth_bals.iter().map(|b| wei_to_f64(*b)).sum();
                let total_token_eth: f64 = tok_bals
                    .iter()
                    .map(|b| u256_to_f64(*b) / decimals_factor * token_price_eth)
                    .sum();
                let total = total_eth + total_token_eth;
                if total > 0.0 { total_token_eth / total } else { 0.5 }
            } else {
                0.5
            };

            // Buy when token ratio < 60%, sell when >= 60%
            let is_buy = token_ratio < 0.60;

            // Log per-wallet balances for debugging
            if let (Some(eth_bals), Some(tok_bals)) = (&eth_balances, &token_balances) {
                for (i, wallet) in task.wallets.iter().enumerate() {
                    let eth_bal = eth_bals.get(i).map(|b| wei_to_f64(*b)).unwrap_or(0.0);
                    let token_bal = tok_bals.get(i).map(|b| u256_to_f64(*b) / decimals_factor).unwrap_or(0.0);
                    tracing::debug!(
                        "[EVM] Project {} | wallet[{}] {} | ETH={:.6} tokens={:.4}",
                        project_id, i, wallet.address, eth_bal, token_bal,
                    );
                }
            }

            tracing::info!(
                "[EVM] Project {} | token_ratio={:.1}% is_buy={} bundle_enabled={} slippage={:.1}%",
                project_id, token_ratio * 100.0, is_buy, project.bundle_enabled.unwrap_or(false),
                project.slippage.unwrap_or(10.0),
            );

            // --- Slippage calculation ---
            let slippage_factor = project.slippage
                .map(|pct| 1.0 - (pct / 100.0))
                .unwrap_or(0.90);

            let mut tokens_for_trade = if token_price_eth > 0.0 {
                trade_eth / token_price_eth
            } else {
                0.0
            };

            let mut buy_expected_tokens = tokens_for_trade * slippage_factor;
            let mut sell_expected_eth = tokens_for_trade * token_price_eth * slippage_factor;

            tracing::info!(
                "[EVM] Project {} | trade_eth={:.6} value_usdc=${:.2} eth_price=${:.2} token_price_eth={:.10} tokens_for_trade={:.4} buy_exp_tokens={:.4} sell_exp_eth={:.6}",
                project_id, trade_eth, value_usdc, eth_price, token_price_eth, tokens_for_trade, buy_expected_tokens, sell_expected_eth,
            );

            // --- Market impact check ---
            let amount_in = if is_buy {
                f64_to_wei(trade_eth)
            } else {
                f64_to_token_units(tokens_for_trade, decimals)
            };

            // Use max_market_impact_bps if set, otherwise fall back to project fee (e.g. 0.3% fee = 30 bps)
            let fee_bps = project.max_market_impact_bps.map(|v| v as u16).unwrap_or((big_int_to_f64(project.fees.clone()) * 100.0) as u16);

            if let Some(ref pkp) = v4_pool_key_params {
                match calculate_market_impact_v4(
                    network.clone(),
                    pkp.clone(),
                    amount_in,
                    is_buy,
                    token_price_eth,
                    decimals,
                )
                .await
                {
                    Ok(impact_bps) => {
                        if impact_bps > fee_bps {
                            tracing::warn!(
                                "[EVM] Project {} trade rejected: {}bps impact > {}bps fee",
                                project_id,
                                impact_bps,
                                fee_bps
                            );

                            let msg = format!(
                                "Trade skipped for project {} - {}bps impact exceeds {}bps fee. Retrying next interval.",
                                project_id, impact_bps, fee_bps
                            );
                            let _ = nc_clone
                                .publish(
                                    subjects::discord::MESSAGE,
                                    msg.into(),
                                )
                                .await;

                            tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                            continue;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[EVM] Project {} market impact check failed: {}. Proceeding.",
                            project_id,
                            e
                        );
                    }
                }
            } else if best_pool.version == 2 || best_pool.version == 3 {
                // Simple liquidity-based impact estimate for V2/V3
                let pool_liquidity_eth = wei_to_f64(best_pool.balance);
                if pool_liquidity_eth > 0.0 {
                    let impact_bps = estimate_impact_from_liquidity(trade_eth, pool_liquidity_eth);
                    if impact_bps > fee_bps {
                        tracing::warn!(
                            "[EVM] Project {} V{} trade rejected: ~{}bps impact > {}bps fee",
                            project_id,
                            best_pool.version,
                            impact_bps,
                            fee_bps
                        );

                        let msg = format!(
                            "Trade skipped for project {} - ~{}bps impact exceeds {}bps fee (V{}). Retrying next interval.",
                            project_id, impact_bps, fee_bps, best_pool.version
                        );
                        let _ = nc_clone
                            .publish(
                                subjects::discord::MESSAGE,
                                msg.into(),
                            )
                            .await;

                        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                        continue;
                    }
                }
            }

            // --- Bundle mode: dispatch BundleBuySell (buy+sell in one tx/bundle) ---
            let bundle_enabled = project.bundle_enabled.unwrap_or(false);
            // Bundle generates 2 legs (buy+sell), both count as on-chain volume.
            // Halve trade size so buy_half + sell_half = full iteration budget.
            if bundle_enabled {
                trade_eth /= 2.0;
                volume_per_interval_usdc = trade_eth * eth_price;
                tokens_for_trade = if token_price_eth > 0.0 { trade_eth / token_price_eth } else { 0.0 };
                buy_expected_tokens = tokens_for_trade * slippage_factor;
                sell_expected_eth = tokens_for_trade * token_price_eth * slippage_factor;
            }
            let bundle_dispatched = if bundle_enabled {
                if let (Some(eth_bals), Some(tok_bals)) = (&eth_balances, &token_balances) {
                    // Bundle: wallet only needs enough ETH to buy — the buy leg acquires tokens
                    // that the sell leg then sells back atomically in the same tx/bundle
                    let mut bundle_candidates: Vec<usize> = vec![];
                    for (i, _) in task.wallets.iter().enumerate() {
                        let eth_bal = eth_bals.get(i).map(|b| wei_to_f64(*b)).unwrap_or(0.0);
                        if eth_bal - 0.008 >= trade_eth / slippage_factor {
                            bundle_candidates.push(i);
                        }
                    }

                    tracing::info!(
                        "[EVM] Project {} | BUNDLE candidates={}/{} wallets (need {:.6} ETH after 0.008 reserve, slippage_factor={:.2})",
                        project_id, bundle_candidates.len(), task.wallets.len(),
                        trade_eth / slippage_factor, slippage_factor,
                    );

                    if !bundle_candidates.is_empty() {
                        let pick = rng.random_range(0..bundle_candidates.len());
                        let wallet = &task.wallets[bundle_candidates[pick]];

                        tracing::info!(
                            "[EVM] Project {} | BUNDLE BUY+SELL | wallet={} | buy={:.6} ETH sell={:.4} tokens | ratio={:.1}%",
                            project_id, wallet.address, trade_eth, tokens_for_trade, token_ratio * 100.0,
                        );

                        // Bundle is atomic: sell exactly what the buy acquires.
                        // buy uses ExactOut (gets exact tokens), sell uses ExactIn (sells same amount).
                        let bundle_task = EvmProcessBundleBuySell {
                            project_id,
                            user_id: project.user_id,
                            token: project.address.clone(),
                            pool: project.pool.clone(),
                            wallet_id: wallet.wallet_id,
                            private_key: wallet.private_key.clone(),
                            address: wallet.address.clone(),
                            buy_exact_tokens: buy_expected_tokens,
                            buy_max_eth: trade_eth,
                            sell_tokens: buy_expected_tokens,
                            sell_min_eth: sell_expected_eth,
                            eth_price,
                            network: project.network.clone(),
                            token_info: token_info_json.clone(),
                            pool_info: pool_info_json.clone(),
                            trade_volume_usdc: volume_per_interval_usdc,
                        };

                        let cmd_bytes = serde_json::to_vec(&StreamInfo {
                            user_id: -1,
                            stream_type: StreamType::EvmBundleBuySell(bundle_task),
                        })
                        .unwrap_or_default();
                        let _ = nc_clone
                            .publish(subjects::cmd::evm::TRADE_BUNDLE_BUY_SELL, cmd_bytes.into())
                            .await;
                        true
                    } else {
                        // Fallback: no wallet can do both sides, do single buy or sell based on ratio
                        if token_ratio < 0.60 {
                            let mut buy_candidates: Vec<usize> = vec![];
                            for (i, _) in task.wallets.iter().enumerate() {
                                let eth_bal = eth_bals.get(i).map(|b| wei_to_f64(*b)).unwrap_or(0.0);
                                if eth_bal - 0.008 >= trade_eth / slippage_factor {
                                    buy_candidates.push(i);
                                }
                            }

                            if !buy_candidates.is_empty() {
                                let pick = rng.random_range(0..buy_candidates.len());
                                let wallet = &task.wallets[buy_candidates[pick]];

                                tracing::info!(
                                    "[EVM] Project {} | BUNDLE FALLBACK BUY | wallet={} | {:.6} ETH | ratio={:.1}%",
                                    project_id, wallet.address, trade_eth, token_ratio * 100.0,
                                );

                                let buy_task = EvmProcessBuy {
                                    project_id,
                                    wallet_id: wallet.wallet_id,
                                    user_id: project.user_id,
                                    token: project.address.clone(),
                                    pool: project.pool.clone(),
                                    value: trade_eth,
                                    tokens: buy_expected_tokens,
                                    private_key: wallet.private_key.clone(),
                                    address: wallet.address.clone(),
                                    eth_price,
                                    network: project.network.clone(),
                                    token_info: token_info_json.clone(),
                                    pool_info: pool_info_json.clone(),
                                    trade_volume_usdc: volume_per_interval_usdc,
                                };

                                let cmd_bytes = serde_json::to_vec(&StreamInfo {
                                    user_id: -1,
                                    stream_type: StreamType::EvmBuy(buy_task),
                                })
                                .unwrap_or_default();
                                let _ = nc_clone
                                    .publish(subjects::cmd::evm::TRADE_BUY, cmd_bytes.into())
                                    .await;
                                true
                            } else {
                                let _ = crate::cache::increment_task_failures(project_id).await;
                                false
                            }
                        } else {
                            let mut sell_candidates: Vec<usize> = vec![];
                            for (i, _) in task.wallets.iter().enumerate() {
                                let token_bal = tok_bals.get(i).map(|b| u256_to_f64(*b) / decimals_factor).unwrap_or(0.0);
                                if token_bal >= tokens_for_trade {
                                    sell_candidates.push(i);
                                }
                            }

                            if !sell_candidates.is_empty() {
                                let pick = rng.random_range(0..sell_candidates.len());
                                let wallet = &task.wallets[sell_candidates[pick]];

                                tracing::info!(
                                    "[EVM] Project {} | BUNDLE FALLBACK SELL | wallet={} | tokens={:.4} | ratio={:.1}%",
                                    project_id, wallet.address, tokens_for_trade, token_ratio * 100.0,
                                );

                                let sell_task = EvmProcessSell {
                                    project_id,
                                    wallet_id: wallet.wallet_id,
                                    user_id: project.user_id,
                                    token: project.address.clone(),
                                    pool: project.pool.clone(),
                                    value: sell_expected_eth,
                                    tokens: tokens_for_trade,
                                    private_key: wallet.private_key.clone(),
                                    address: wallet.address.clone(),
                                    eth_price,
                                    network: project.network.clone(),
                                    token_info: token_info_json.clone(),
                                    pool_info: pool_info_json.clone(),
                                    trade_volume_usdc: volume_per_interval_usdc,
                                };

                                let cmd_bytes = serde_json::to_vec(&StreamInfo {
                                    user_id: -1,
                                    stream_type: StreamType::EvmSell(sell_task),
                                })
                                .unwrap_or_default();
                                let _ = nc_clone
                                    .publish(subjects::cmd::evm::TRADE_SELL, cmd_bytes.into())
                                    .await;
                                true
                            } else {
                                let _ = crate::cache::increment_task_failures(project_id).await;
                                false
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        "[EVM] Project {} | BUNDLE: balance fetch not available",
                        project_id
                    );
                    let _ = crate::cache::increment_task_failures(project_id).await;
                    false
                }
            } else {
                false
            };

            // Bundle was attempted but failed — skip to next interval (no single-trade fallback)
            if bundle_enabled && !bundle_dispatched {
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                continue;
            }

            // --- Single-trade mode (only when bundle not enabled) ---
            if !bundle_dispatched {

            // --- Wallet balance validation + selection ---
            let (wallet, is_buy) = if let (Some(eth_bals), Some(tok_bals)) =
                (&eth_balances, &token_balances)
            {
                let min_trade_usdc = (base_value_usdc * 0.01).max(1.0);
                let min_trade_eth_threshold = min_trade_usdc / eth_price;

                let build_candidates = |buy: bool| -> Vec<(usize, f64, bool)> {
                    task.wallets
                        .iter()
                        .enumerate()
                        .filter_map(|(i, _)| {
                            let available = if buy {
                                eth_bals.get(i).map(|b| wei_to_f64(*b)).unwrap_or(0.0)
                            } else {
                                let token_amount = tok_bals.get(i).map(|b| u256_to_f64(*b) / decimals_factor).unwrap_or(0.0);
                                token_amount * token_price_eth
                            };
                            if available >= min_trade_eth_threshold {
                                Some((i, available, buy))
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                let mut candidates = build_candidates(is_buy);

                if candidates.is_empty() {
                    candidates = build_candidates(!is_buy);
                }

                if candidates.is_empty() {
                    let mut combined = build_candidates(true);
                    combined.extend(build_candidates(false));
                    candidates = combined;
                }

                if candidates.is_empty() {
                    tracing::warn!(
                        "[EVM] Project {} no wallet with sufficient balance for either buy (need={:.6} ETH) or sell (need={:.6} tokens)",
                        project_id,
                        trade_eth,
                        tokens_for_trade
                    );
                    let _ = crate::cache::increment_task_failures(project_id).await;
                    tokio::time::sleep(Duration::from_secs(interval_secs)).await;
                    continue;
                }

                let buy_count = candidates.iter().filter(|c| c.2).count();
                let sell_count = candidates.iter().filter(|c| !c.2).count();

                let pick = rng.random_range(0..candidates.len());
                let (idx, available_eth, direction) = candidates[pick];

                if available_eth < trade_eth {
                    trade_eth = available_eth * 0.95;
                    tokens_for_trade = if token_price_eth > 0.0 { trade_eth / token_price_eth } else { 0.0 };
                    buy_expected_tokens = tokens_for_trade * slippage_factor;
                    sell_expected_eth = tokens_for_trade * token_price_eth * slippage_factor;
                    volume_per_interval_usdc = trade_eth * eth_price;
                }

                let dir = if direction { "buy" } else { "sell" };
                tracing::info!(
                    "[EVM] Project {} {} {:.6} ETH with wallet {} ({} buy/{} sell candidates)",
                    project_id,
                    dir,
                    trade_eth,
                    task.wallets[idx].address,
                    buy_count,
                    sell_count
                );

                (&task.wallets[idx], direction)
            } else {
                let idx = rng.random_range(0..task.wallets.len());
                (&task.wallets[idx], is_buy)
            };

            // --- Publish trade ---
            if is_buy {
                let buy_task = EvmProcessBuy {
                    project_id,
                    wallet_id: wallet.wallet_id,
                    user_id: project.user_id,
                    token: project.address.clone(),
                    pool: project.pool.clone(),
                    value: trade_eth,
                    tokens: buy_expected_tokens,
                    private_key: wallet.private_key.clone(),
                    address: wallet.address.clone(),
                    eth_price,
                    network: project.network.clone(),
                    token_info: token_info_json.clone(),
                    pool_info: pool_info_json.clone(),
                    trade_volume_usdc: volume_per_interval_usdc,
                };

                let cmd_bytes = serde_json::to_vec(&StreamInfo {
                    user_id: -1,
                    stream_type: StreamType::EvmBuy(buy_task),
                })
                .unwrap_or_default();
                let _ = nc_clone
                    .publish(subjects::cmd::evm::TRADE_BUY, cmd_bytes.into())
                    .await;
            } else {
                let sell_task = EvmProcessSell {
                    project_id,
                    wallet_id: wallet.wallet_id,
                    user_id: project.user_id,
                    token: project.address.clone(),
                    pool: project.pool.clone(),
                    value: sell_expected_eth,
                    tokens: tokens_for_trade,
                    private_key: wallet.private_key.clone(),
                    address: wallet.address.clone(),
                    eth_price,
                    network: project.network.clone(),
                    token_info: token_info_json.clone(),
                    pool_info: pool_info_json.clone(),
                    trade_volume_usdc: volume_per_interval_usdc,
                };

                let cmd_bytes = serde_json::to_vec(&StreamInfo {
                    user_id: -1,
                    stream_type: StreamType::EvmSell(sell_task),
                })
                .unwrap_or_default();
                let _ = nc_clone
                    .publish(subjects::cmd::evm::TRADE_SELL, cmd_bytes.into())
                    .await;
            }

            } // end if !bundle_dispatched

            // Broadcast daily volume update
            let current_volume = crate::cache::get_daily_volume(project_id)
                .await
                .unwrap_or(Some(0.0))
                .unwrap_or(0.0);

            let volume_info = DailyVolumeInfo {
                project_id,
                volume_usdc: current_volume,
                target_usdc: daily_volume_usdc,
            };
            let vol_bytes = serde_json::to_vec(&StreamInfo {
                user_id: project.user_id,
                stream_type: StreamType::DailyVolumeUpdate(volume_info),
            })
            .unwrap_or_default();
            let _ = nc_clone
                .publish(subjects::events::evm::DAILY_VOLUME, vol_bytes.into())
                .await;

            // Sleep with jitter ±20%, refreshing heartbeat every 10s
            let jitter = rng.random_range(0.8..1.2);
            let total_sleep = Duration::from_secs((interval_secs as f64 * jitter) as u64);
            let sleep_start = tokio::time::Instant::now();
            let mut shutdown_received = false;
            while sleep_start.elapsed() < total_sleep {
                let remaining = total_sleep.saturating_sub(sleep_start.elapsed());
                let tick = Duration::from_secs(10).min(remaining);
                tokio::select! {
                    _ = tokio::time::sleep(tick) => {
                        let _ = vm_redis::task_heartbeat::refresh_heartbeat("evm", project_id).await;
                    }
                    _ = shutdown.cancelled() => {
                        tracing::info!("[EVM] Shutdown signal received during sleep, stopping volume maker for project {}", project_id);
                        shutdown_received = true;
                        break;
                    }
                }
            }
            if shutdown_received {
                break;
            }
        }
    });

    Ok(())
}
