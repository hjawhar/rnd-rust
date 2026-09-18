use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

use chrono::{Timelike, Utc};
use vm_data::utils::helpers::{
    big_int_to_f64, modify_by_random_percent, mul_f64_and_u64_to_u64,
    random_between_i32, shuffle_array,
};
use vm_solana::utils::helpers::fetch_sol_price;
use vm_data::models::{
    streams::{DailyVolumeInfo, StreamInfo, StreamType, TaskStatusInfo},
    task::{ProcessBundleBuySell, ProcessBuy, ProcessSell, ProcessTask, ProcessWalletInfo},
};
use vm_nats;
use vm_nats::subjects;

use vm_redis::task_state::MAX_FAILURES;
use vm_volume::distribution::{cap_trade_to_daily_budget, get_volume_multiplier};

use solana_native_token::LAMPORTS_PER_SOL;
use rand::Rng;
use tokio::sync::mpsc::Sender;

use crate::cache::price_cache::{add_sol_price, get_sol_price};
use crate::state::get_state;
use vm_solana::rpc::batched_rpc::{get_multiple_accounts_batched, extract_token_balance};
use solana_pubkey::Pubkey;
use spl_associated_token_account::get_associated_token_address_with_program_id;

/// Sleeps for the given duration while refreshing the task heartbeat every 10s.
/// Returns `false` if the task was removed from cache or ownership lost/superseded,
/// `true` if the full duration elapsed normally.
async fn sleep_with_heartbeat(project_id: i32, generation: u64, worker_id: &str, total: Duration) -> bool {
    let state = get_state();
    let tick = Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    loop {
        let elapsed = start.elapsed();
        if elapsed >= total {
            return true;
        }
        let remaining = total - elapsed;
        tokio::time::sleep(tick.min(remaining)).await;

        // Check if task was stopped locally
        if state.get_task(project_id).is_none() {
            return false;
        }

        // Check ownership + generation from Redis
        match vm_redis::task_ownership::get_owner("sol", project_id).await {
            Ok(Some(o)) if o.worker_id == worker_id && o.generation == generation => {}
            _ => return false,
        }

        // Refresh heartbeat
        let _ = vm_redis::task_heartbeat::refresh_heartbeat("sol", project_id).await;
    }
}

pub async fn start_volume_maker(
    task: ProcessTask,
    tx: Arc<Sender<StreamInfo>>,
    nats_client: async_nats::Client,
    db: vm_data::db::Database,
    generation: u64,
    worker_id: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let state = get_state();
    let project_id = task.project.id;
    let interval_secs = big_int_to_f64(task.project.trading_interval.clone()) as u64;

    if state.get_task(project_id).is_none() {
        tracing::warn!("[VM] Task not found in cache for project {}, exiting", project_id);
        return Ok(());
    }

    tracing::info!(
        "[VM] Project {} | Starting volume maker loop (gen={}, interval={}s)",
        project_id, generation, interval_secs,
    );

    let mut iteration: u64 = 0;
    let mut last_dispatch = tokio::time::Instant::now();

    loop {
        // Check ownership + generation from Redis
        match vm_redis::task_ownership::get_owner("sol", project_id).await {
            Ok(Some(o)) if o.worker_id == worker_id && o.generation == generation => {}
            Ok(Some(o)) => {
                tracing::info!(
                    "[VM] Project {} | gen={} exiting (superseded: owner={}, gen={})",
                    project_id, generation, o.worker_id, o.generation,
                );
                return Ok(());
            }
            Ok(None) => {
                tracing::info!("[VM] Project {} | gen={} exiting (ownership released)", project_id, generation);
                return Ok(());
            }
            Err(e) => {
                tracing::error!("[VM] Project {} | gen={} ownership check failed: {}", project_id, generation, e);
                return Ok(());
            }
        }

        // Also check local task cache
        if state.get_task(project_id).is_none() {
            tracing::info!("[VM] Project {} | gen={} exiting (removed from local cache)", project_id, generation);
            return Ok(());
        }

        iteration += 1;
        let since_last = last_dispatch.elapsed().as_secs_f64();
        if iteration > 1 && since_last < (interval_secs as f64 * 0.5) {
            tracing::warn!(
                "[VM] Project {} | gen={} iter={} | DUPLICATE? elapsed={:.1}s < interval={}s",
                project_id, generation, iteration, since_last, interval_secs,
            );
        }

        // Refresh task heartbeat
        let _ = vm_redis::task_heartbeat::refresh_heartbeat("sol", project_id).await;

        last_dispatch = tokio::time::Instant::now();

        // Run one iteration; on error, log and continue to retry next interval
        match run_iteration(&task, &tx, &nats_client, &db, generation, iteration).await {
            Ok(()) => {
                tracing::info!("[VM] Project {} | gen={} iter={} | iteration OK", project_id, generation, iteration);
            }
            Err(e) => {
                tracing::error!("[VM] Project {} | gen={} iter={} | error: {}", project_id, generation, iteration, e);
            }
        }

        // Sleep for the configured interval with ±20% jitter
        let jitter = rand::rng().random_range(0.8..1.2);
        let sleep_duration = Duration::from_secs((interval_secs as f64 * jitter) as u64);
        if !sleep_with_heartbeat(project_id, generation, &worker_id, sleep_duration).await {
            tracing::info!(
                "[VM] Project {} | gen={} exiting (stopped or superseded during sleep)",
                project_id, generation,
            );
            return Ok(());
        }
    }
}

async fn run_iteration(
    task: &ProcessTask,
    tx: &Arc<Sender<StreamInfo>>,
    nats_client: &async_nats::Client,
    db: &vm_data::db::Database,
    generation: u64,
    iteration: u64,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let state = get_state();
    let project_id = task.project.id;

    let frequency = 86400.0 / big_int_to_f64(task.project.trading_interval.clone());
    let base_value_usdc = big_int_to_f64(task.project.trading_daily_volume.clone()) / frequency;

    // Apply organic volume distribution based on hour of day
    let current_hour = Utc::now().hour();
    let volume_multiplier = get_volume_multiplier(current_hour);
    let proposed_value_usdc = base_value_usdc * volume_multiplier;

    // Check daily volume budget and cap if needed
    let daily_target_usdc = big_int_to_f64(task.project.trading_daily_volume.clone());
    let remaining_budget =
        vm_redis::task_state::get_daily_volume_remaining("sol", project_id, daily_target_usdc)
            .await
            .unwrap_or(daily_target_usdc);

    // Minimum trade threshold: $1 or 1% of base value, whichever is larger
    let min_trade_threshold = (base_value_usdc * 0.01).max(1.0);
    let capped = cap_trade_to_daily_budget(proposed_value_usdc, remaining_budget, min_trade_threshold);

    // Skip trade if daily budget is exhausted
    if capped.skip_trade {
        tracing::debug!("[VM] Project {} | skipping (daily budget exhausted)", project_id);
        return Ok(());
    }

    let value_usdc = capped.value_usdc;

    let sol_price = get_sol_price().await;
    let sol_price = match sol_price {
        Ok(sol_price) => sol_price,
        Err(e) => return Err(format!("Failed to fetch sol price from redis cache: {}", e).into()),
    };

    let sol_price = match sol_price {
        Some(sol_price) => sol_price,
        None => {
            let fetched_sol_price = fetch_sol_price().await;
            if let Ok(fetched_sol_price) = fetched_sol_price {
                let _ = add_sol_price(fetched_sol_price).await;
                fetched_sol_price
            } else {
                return Err("Failed to fetch sol price from binance".into());
            }
        }
    };

    let value = value_usdc / sol_price;

    if task.wallets.is_empty() {
        tracing::debug!("[VM] Project {} | skipping (no wallets)", project_id);
        return Ok(());
    }

    let slippage_factor = task.project.slippage
        .map(|pct| 1.0 - (pct / 100.0))
        .unwrap_or(0.90);

    let mut buy_wallets: Vec<ProcessWalletInfo> = vec![];
    let mut sell_wallets: Vec<ProcessWalletInfo> = vec![];
    let market_pair = state.fetch_pair(task.project.pool.clone()).await;
    let market_pair = match market_pair {
        Ok(market_pair) => market_pair,
        Err(_err) => {
            return Err("Failed to fetch market pair".into());
        }
    };
    let market_pair = match market_pair {
        Some(market_pair) => market_pair,
        None => {
            return Err("Failed to find market pair".into());
        }
    };

    let bundle_enabled = task.project.bundle_enabled.unwrap_or(false);
    // Bundle generates 2 legs (buy+sell), both count as on-chain volume.
    // Halve the trade size so buy_half + sell_half = full iteration budget.
    let send_value = {
        let v = modify_by_random_percent(value, 3.0, 8.0);
        if bundle_enabled { v / 2.0 } else { v }
    };
    let required_balance = mul_f64_and_u64_to_u64(send_value, LAMPORTS_PER_SOL);

    let token_decimals = task.project.decimals.unwrap_or(6) as u8;
    let token_price = state.get_token_price(task.project.pool.clone(), token_decimals).await;

    let token_price = match token_price {
        Ok(token_price) => token_price,
        Err(err) => {
            return Err(format!("Failed to fetch token price: {}", err).into());
        }
    };

    let token_info = match state.fetch_token_info(&task.project.address).await.ok() {
        Some(token_info) => token_info,
        None => {
            return Err("Failed to get token info".into());
        }
    };

    let decimals = 10_i32.pow(token_info.decimals as u32) as u64;
    let tokens_out = ((send_value as f64 / token_price) * decimals as f64).floor();

    // Fetch live balances via RPC (not cache) to avoid stale data
    let rpc = state.rpc.clone();
    let mint = Pubkey::from_str_const(&task.project.address);

    // Detect token program (Token vs Token-2022) for correct ATA derivation
    let token_program = state.fetch_token_program(&task.project.address).await
        .unwrap_or(spl_token::ID);

    // Build list of all pubkeys: wallet addresses (SOL) + ATAs (token)
    let mut wallet_pubkeys: Vec<Pubkey> = Vec::with_capacity(task.wallets.len());
    let mut ata_pubkeys: Vec<Pubkey> = Vec::with_capacity(task.wallets.len());
    for wallet in task.wallets.iter() {
        let wallet_pk = Pubkey::from_str_const(&wallet.address);
        wallet_pubkeys.push(wallet_pk);
        ata_pubkeys.push(get_associated_token_address_with_program_id(&wallet_pk, &mint, &token_program));
    }

    // Batch fetch SOL balances + token ATA accounts in parallel (1 RPC call each for ≤50 wallets)
    let all_pubkeys: Vec<Pubkey> = wallet_pubkeys.iter().chain(ata_pubkeys.iter()).cloned().collect();
    let accounts = match get_multiple_accounts_batched(rpc.clone(), &all_pubkeys).await {
        Ok(accs) => accs,
        Err(e) => {
            tracing::warn!("[VM] Project {} failed to fetch live balances: {}", task.project.id, e);
            return Err(e);
        }
    };

    let mut balances_sol = HashMap::new();
    let mut balances_tokens = HashMap::new();
    for (i, wallet) in task.wallets.iter().enumerate() {
        let sol_bal = accounts
            .get(&wallet_pubkeys[i])
            .and_then(|a| a.as_ref())
            .map(|a| a.lamports as f64 / LAMPORTS_PER_SOL as f64)
            .unwrap_or(0.0);
        let token_bal = accounts
            .get(&ata_pubkeys[i])
            .and_then(|a| a.as_ref())
            .and_then(extract_token_balance)
            .unwrap_or(0) as f64;

        balances_sol.insert(wallet.address.clone(), sol_bal);
        balances_tokens.insert(wallet.address.clone(), token_bal as i64);

        let can_buy = (mul_f64_and_u64_to_u64(sol_bal, LAMPORTS_PER_SOL) as i64
            - required_balance as i64)
            - mul_f64_and_u64_to_u64(0.006, LAMPORTS_PER_SOL) as i64
            > 0;
        if can_buy {
            buy_wallets.push(wallet.clone());
        }

        let can_sell = token_bal > tokens_out;
        if can_sell {
            sell_wallets.push(wallet.clone());
        }
    }

    let buy_wallets = shuffle_array(&buy_wallets)?;
    let sell_wallets = shuffle_array(&sell_wallets)?;

    // Portfolio ratio — used by both bundle and single-trade modes
    let total_sol_value: f64 = balances_sol.values().sum();
    let total_token_balance: i64 = balances_tokens.values().sum();
    let total_token_value: f64 =
        (total_token_balance as f64 / decimals as f64) * token_price;

    let total_value = total_sol_value + total_token_value;
    let token_ratio = if total_value > 0.0 {
        total_token_value / total_value
    } else {
        0.5
    };

    // Buy when token ratio < 60%, sell when >= 60%
    let mut is_buy = token_ratio < 0.60;

    let max_impact = Some(task.project.max_market_impact_bps.map(|v| v as u16).unwrap_or((big_int_to_f64(task.project.fees.clone()) * 100.0) as u16));
    let trade_volume_usdc = send_value * sol_price;
    let market_pair_json = serde_json::to_string(&market_pair).unwrap_or_default();
    let jito_tip = task.project.jito_tip.unwrap_or(0.00002);
    tracing::info!(
        "[VM] Project {} | gen={} iter={} | bundle={} | token_ratio={:.1}% | wallets={} | slippage={:.1}%",
        task.project.id,
        generation, iteration,
        bundle_enabled,
        token_ratio * 100.0,
        task.wallets.len(),
        task.project.slippage.unwrap_or(10.0),
    );

    // Cap buy value to wallet's actual available balance.
    // Exact output mode uses 1.2x buffer + TOKEN_ACCOUNT_RENT (~0.002 SOL) + tx fees.
    // Reserve 0.01 SOL for rent + fees, divide by 1.2 for exact_output headroom.
    let cap_buy_value = |wallet_addr: &str, value: f64| -> (f64, f64) {
        let sol_bal = balances_sol.get(wallet_addr).copied().unwrap_or(0.0);
        let max_buy = ((sol_bal - 0.01).max(0.0)) / 1.2;
        let capped = value.min(max_buy);
        let capped_tokens = if capped < value && value > 0.0 {
            (tokens_out * (capped / value)).floor()
        } else {
            tokens_out
        };
        (capped, capped_tokens)
    };

    if bundle_enabled {
        // Bundle mode: wallet only needs enough SOL to buy — the buy leg acquires tokens
        // that the sell leg then sells back atomically in the same Jito bundle
        let bundle_candidates: Vec<&ProcessWalletInfo> = buy_wallets.iter().collect();

        let bundle_success = if !bundle_candidates.is_empty() {
            let random_idx = random_between_i32(0, bundle_candidates.len() as i32 - 1).unwrap_or(0);
            let wallet = bundle_candidates[random_idx as usize];

            let (buy_value, buy_tokens) = cap_buy_value(&wallet.address, send_value);
            let buy_volume_usdc = buy_value * sol_price;

            tracing::info!(
                "[VM] Project {} | gen={} iter={} | BUNDLE BUY+SELL | wallet={} | buy={:.4} SOL tokens={:.0} | ratio={:.1}%",
                task.project.id, generation, iteration, wallet.address, buy_value, buy_tokens, token_ratio * 100.0,
            );

            // Bundle is atomic — no slippage between buy and sell.
            // buy_value = SOL to spend, buy_min_tokens = exact tokens wanted,
            // sell_tokens = same tokens, sell_min_value not used (set to 0).
            let _ = tx
                .send(StreamInfo {
                    user_id: task.project.user_id,
                    stream_type: StreamType::BundleBuySell(ProcessBundleBuySell {
                        market_pair: market_pair_json.clone(),
                        project_id: task.project.id,
                        token: task.project.address.clone(),
                        pool: task.project.pool.clone(),
                        buy_wallet_id: wallet.wallet_id,
                        buy_private_key: wallet.private_key.clone(),
                        buy_address: wallet.address.clone(),
                        buy_value,
                        buy_min_tokens: buy_tokens,
                        sell_wallet_id: wallet.wallet_id,
                        sell_private_key: wallet.private_key.clone(),
                        sell_address: wallet.address.clone(),
                        sell_min_value: 0.0,
                        sell_tokens: buy_tokens,
                        sol_price,
                        max_market_impact_bps: max_impact,
                        trade_volume_usdc: buy_volume_usdc,
                        jito_tip,
                        token_decimals,
                    }),
                })
                .await;
            true
        } else {
            // Fallback: no wallet can do both sides, do single buy or sell based on ratio
            if token_ratio < 0.60 {
                if !buy_wallets.is_empty() {
                    let random_idx = random_between_i32(0, buy_wallets.len() as i32 - 1).unwrap_or(0);
                    let wallet = &buy_wallets[random_idx as usize];
                    let (buy_value, buy_tokens) = cap_buy_value(&wallet.address, send_value);

                    tracing::info!(
                        "[VM] Project {} | gen={} iter={} | BUNDLE FALLBACK BUY | wallet={} | {:.4} SOL | ratio={:.1}%",
                        task.project.id, generation, iteration, wallet.address, buy_value, token_ratio * 100.0,
                    );

                    let _ = tx
                        .send(StreamInfo {
                            user_id: task.project.user_id,
                            stream_type: StreamType::Buy(ProcessBuy {
                                market_pair: market_pair_json.clone(),
                                project_id: task.project.id,
                                wallet_id: wallet.wallet_id,
                                token: task.project.address.clone(),
                                pool: task.project.pool.clone(),
                                value: buy_value,
                                private_key: wallet.private_key.clone(),
                                address: wallet.address.clone(),
                                tokens: buy_tokens * slippage_factor,
                                sol_price,
                                max_market_impact_bps: max_impact,
                                trade_volume_usdc: buy_value * sol_price,
                                jito_tip,
                            }),
                        })
                        .await;
                    true
                } else {
                    false
                }
            } else if !sell_wallets.is_empty() {
                let random_idx = random_between_i32(0, sell_wallets.len() as i32 - 1).unwrap_or(0);
                let wallet = &sell_wallets[random_idx as usize];

                tracing::info!(
                    "[VM] Project {} | gen={} iter={} | BUNDLE FALLBACK SELL | wallet={} | tokens={:.0} | ratio={:.1}%",
                    task.project.id, generation, iteration, wallet.address, tokens_out, token_ratio * 100.0,
                );

                let _ = tx
                    .send(StreamInfo {
                        user_id: task.project.user_id,
                        stream_type: StreamType::Sell(ProcessSell {
                            market_pair: market_pair_json.clone(),
                            project_id: task.project.id,
                            wallet_id: wallet.wallet_id,
                            token: task.project.address.clone(),
                            pool: task.project.pool.clone(),
                            value: send_value * slippage_factor,
                            private_key: wallet.private_key.clone(),
                            address: wallet.address.clone(),
                            tokens: tokens_out,
                            sol_price,
                            max_market_impact_bps: max_impact,
                            trade_volume_usdc,
                            jito_tip,
                        }),
                    })
                    .await;
                true
            } else {
                false
            }
        };

        if bundle_success {
            let _ = vm_redis::task_state::reset_task_failures("sol", task.project.id).await;
        } else {
            let task_failures = vm_redis::task_state::increment_task_failures("sol", task.project.id)
                .await.unwrap_or(1);

            if task_failures >= MAX_FAILURES {
                auto_stop_task(task, state, nats_client, db).await;
                return Err("Auto-stopped after max failures".into());
            }
        }
    } else {
        // Single-trade mode (no bundle)
        let mut wallet: Option<ProcessWalletInfo> = None;
        if is_buy && !buy_wallets.is_empty() {
            let random_idx = random_between_i32(0, buy_wallets.len() as i32 - 1).unwrap_or(0);
            wallet = Some(buy_wallets[random_idx as usize].clone());
        } else if !is_buy && !sell_wallets.is_empty() {
            let random_idx = random_between_i32(0, sell_wallets.len() as i32 - 1).unwrap_or(0);
            wallet = Some(sell_wallets[random_idx as usize].clone());
        } else if !buy_wallets.is_empty() {
            // Fallback: wanted sell but no sell wallets, buy instead
            let random_idx = random_between_i32(0, buy_wallets.len() as i32 - 1).unwrap_or(0);
            wallet = Some(buy_wallets[random_idx as usize].clone());
            is_buy = true;
        } else if !sell_wallets.is_empty() {
            // Fallback: wanted buy but no buy wallets, sell instead
            let random_idx = random_between_i32(0, sell_wallets.len() as i32 - 1).unwrap_or(0);
            wallet = Some(sell_wallets[random_idx as usize].clone());
            is_buy = false;
        }

        if wallet.is_none() {
            let task_failures = vm_redis::task_state::increment_task_failures("sol", task.project.id)
                .await.unwrap_or(1);
            if task_failures >= MAX_FAILURES {
                auto_stop_task(task, state, nats_client, db).await;
                return Err("Auto-stopped after max failures".into());
            }
            return Ok(());
        }

        let _ = vm_redis::task_state::reset_task_failures("sol", task.project.id).await;
        let wallet = wallet.unwrap();

        // Cap buy value to wallet's available balance
        let (actual_value, actual_tokens) = if is_buy {
            cap_buy_value(&wallet.address, send_value)
        } else {
            (send_value, tokens_out)
        };
        let actual_volume_usdc = actual_value * sol_price;

        tracing::info!(
            "[VM] Project {} | gen={} iter={} | SINGLE {} | wallet={} | value={:.4} SOL (${:.2}) | sol_price=${:.2}",
            task.project.id,
            generation, iteration,
            if is_buy { "BUY" } else { "SELL" },
            wallet.address,
            actual_value,
            actual_volume_usdc,
            sol_price,
        );

        if is_buy {
            let _ = tx
                .send(StreamInfo {
                    user_id: task.project.user_id,
                    stream_type: StreamType::Buy(ProcessBuy {
                        market_pair: market_pair_json,
                        project_id: task.project.id,
                        wallet_id: wallet.wallet_id,
                        token: task.project.address.clone(),
                        pool: task.project.pool.clone(),
                        value: actual_value,
                        private_key: wallet.private_key.clone(),
                        address: wallet.address.clone(),
                        tokens: actual_tokens * slippage_factor,
                        sol_price,
                        max_market_impact_bps: max_impact,
                        trade_volume_usdc: actual_volume_usdc,
                        jito_tip,
                    }),
                })
                .await;
        } else {
            let _ = tx
                .send(StreamInfo {
                    user_id: task.project.user_id,
                    stream_type: StreamType::Sell(ProcessSell {
                        market_pair: market_pair_json,
                        project_id: task.project.id,
                        wallet_id: wallet.wallet_id,
                        token: task.project.address.clone(),
                        pool: task.project.pool.clone(),
                        value: send_value * slippage_factor,
                        private_key: wallet.private_key.clone(),
                        address: wallet.address.clone(),
                        tokens: tokens_out,
                        sol_price,
                        max_market_impact_bps: max_impact,
                        trade_volume_usdc,
                        jito_tip,
                    }),
                })
                .await;
        }
    }

    // Broadcast daily volume update via NATS
    let daily_target_usdc_val = big_int_to_f64(task.project.trading_daily_volume.clone());
    let current_volume = vm_redis::task_state::get_daily_volume_remaining("sol", project_id, daily_target_usdc_val)
        .await
        .map(|remaining| daily_target_usdc_val - remaining)
        .unwrap_or(0.0);
    let volume_info = DailyVolumeInfo {
        project_id,
        volume_usdc: current_volume,
        target_usdc: daily_target_usdc_val,
    };
    let vol_bytes = serde_json::to_vec(&StreamInfo {
        user_id: task.project.user_id,
        stream_type: StreamType::DailyVolumeUpdate(volume_info),
    })
    .unwrap_or_default();
    let _ = nats_client
        .publish(subjects::events::sol::DAILY_VOLUME, vol_bytes.into())
        .await;

    Ok(())
}

/// Auto-stop a task after too many failures: clear caches, update DB, notify Discord + WS.
async fn auto_stop_task(
    task: &ProcessTask,
    state: &crate::state::AppState,
    nats_client: &async_nats::Client,
    db: &vm_data::db::Database,
) {
    let _ = vm_redis::task_state::reset_task_failures("sol", task.project.id).await;
    let _ = vm_redis::task_ownership::release_task("sol", task.project.id, &state.worker_id).await;
    state.remove_task(task.project.id);

    let msg = format!(
        "Stopping task {} with ID {} for token {} - pool: {} ({} - {}%) - after {} retries",
        task.project.name.clone().unwrap_or("Unknown".to_string()),
        task.project.id,
        task.project.address,
        task.project.pool,
        task.project.pool_type,
        task.project.fees,
        vm_redis::task_state::MAX_FAILURES,
    );

    let _ = nats_client
        .publish(vm_nats::subjects::discord::MESSAGE, msg.into())
        .await;

    let _ = vm_redis::task_heartbeat::clear_heartbeat("sol", task.project.id).await;
    let _ = db.update_project_status(task.project.id, "stopped").await;
    let status_bytes = serde_json::to_vec(&StreamInfo {
        user_id: task.project.user_id,
        stream_type: StreamType::TaskStatusUpdate(TaskStatusInfo {
            project_id: task.project.id,
            status: "stopped".to_string(),
        }),
    })
    .unwrap_or_default();
    let _ = nats_client
        .publish(vm_nats::subjects::events::sol::TASK_STATUS, status_bytes.into())
        .await;
}
