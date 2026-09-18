use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_data::models::task::{EvmProcessBundleBuySell, EvmProcessBuy, EvmProcessSell};
use vm_evm::constants::{MAX_UINT_HEX, Network};
use vm_evm::contracts::{
    ERC20Contract, Permit2, UniswapV4PositionManager, UniswapV4UniversalRouter,
};
use vm_data::utils::helpers::f64_to_big_int;
use vm_evm::helpers::{f64_to_token_units, f64_to_wei, str_to_pk, u256_to_f64};
use vm_evm::models::token::{CustomPool, CustomToken};
use vm_evm::simulation::uniswap::{
    encode_multihop_exact_in_path, encode_settle_all, encode_sweep, encode_swap_exact_in,
    encode_swap_exact_out, encode_take_all, CommandType, PoolKey, RoutePlanner, SwapExactIn,
    SwapExactOut, TxWatcherInfo, V4Action, V4Planner,
};
use vm_nats::subjects;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::network::{Ethereum, EthereumWallet, TransactionBuilder};
use alloy::primitives::{Address, Bytes, FixedBytes, U160, U256, aliases::U48};
use alloy::providers::{PendingTransactionBuilder, Provider};
use alloy::rpc::types::TransactionRequest;
use vm_data::models::transaction::NewTransaction;
use std::time::Duration;

use super::buy::process_buy;
use super::sell::process_sell;

pub async fn process_bundle_buy_sell(
    nc: async_nats::Client,
    db: Database,
    task: EvmProcessBundleBuySell,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let pool: CustomPool = serde_json::from_str(&task.pool_info)?;

    if pool.version == 4 {
        process_v4_atomic(nc, db, task, pool).await
    } else {
        process_v2v3_sequential(nc, db, task).await
    }
}

/// V4 atomic path: both buy+sell in a single Universal Router execute_1() tx
async fn process_v4_atomic(
    nc: async_nats::Client,
    db: Database,
    task: EvmProcessBundleBuySell,
    pool: CustomPool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::from_network_name(&task.network).ok_or("Unsupported network")?;
    let token: CustomToken = serde_json::from_str(&task.token_info)?;
    let project_id = task.project_id;

    tracing::info!(
        "[EVM] Bundle V4: project={} wallet={} buy_tokens={:.4} max_eth={:.6} sell_tokens={:.4} min_eth={:.6}",
        project_id,
        task.address,
        task.buy_exact_tokens,
        task.buy_max_eth,
        task.sell_tokens,
        task.sell_min_eth,
    );

    let provider = network.get_provider();
    let signer = str_to_pk(&task.private_key)?;

    let deadline = U256::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs()
            + 100,
    );

    // Parallel nonce + gas fetch
    let is_avalanche = network.clone() as i32 == Network::Avalanche as i32;
    let (nonce_result, gas_result) = tokio::join!(
        provider.get_transaction_count(signer.address()),
        async {
            if is_avalanche {
                provider.get_gas_price().await.map(|p| (p, p))
            } else {
                provider
                    .estimate_eip1559_fees()
                    .await
                    .map(|f| (f.max_fee_per_gas, f.max_priority_fee_per_gas))
            }
        }
    );
    let mut nonce = nonce_result?;
    let (max_fee_per_gas, max_priority_fee_per_gas) = gas_result?;

    // Contract addresses
    let universal_router_address: Address = match network {
        Network::Ethereum => Address::from_str("0x66a9893cc07d91d95644aedd05d03f95e1dba8af")?,
        Network::Base => Address::from_str("0x6fF5693b99212Da76ad316178A184AB56D299b43")?,
        _ => Address::from_str("0x6fF5693b99212Da76ad316178A184AB56D299b43")?,
    };
    let position_manager_address: Address = match network {
        Network::Ethereum => Address::from_str("0xbd216513d74c8cf14cf4747e6aaa6420ff64ee9e")?,
        Network::Base => Address::from_str("7c5f5a4bbd8fd63184577525326123b519429bdc")?,
        _ => Address::from_str("7c5f5a4bbd8fd63184577525326123b519429bdc")?,
    };
    let permit2_address = Address::from_str("0x000000000022D473030F116dDEE9F6B43aC78BA3")?;

    let universal_router =
        UniswapV4UniversalRouter::new(universal_router_address, &provider);
    let position_manager =
        UniswapV4PositionManager::new(position_manager_address, provider.clone());
    let permit_contract = Permit2::new(permit2_address, provider.clone());
    let erc20_contract = ERC20Contract::new(token.token, provider.clone());

    // Resolve V4 pool key from PositionManager
    let pool_str: String = pool.pool.chars().take(52).collect();
    let pool_keys = position_manager
        .poolKeys(FixedBytes::from_str(&pool_str)?)
        .call()
        .await?;

    let token_pool_key = PoolKey {
        currency0: pool_keys.currency0,
        currency1: pool_keys.currency1,
        fee: pool_keys.fee.to::<u32>(),
        tick_spacing: pool_keys.tickSpacing.to_string().parse::<i32>()?,
        hooks: pool_keys.hooks,
    };

    // Build pre-approval txs for sell leg (token → Permit2 → UniversalRouter)
    let mut build_requests: Vec<(Address, Address, i32, Bytes, U256, u64, String)> = Vec::new();

    let sell_tokens_u256 = f64_to_token_units(task.sell_tokens, token.decimals);

    // Check ERC20 → Permit2 approval (for sell leg)
    let token_to_permit2_allowance = erc20_contract
        .allowance(signer.address(), permit2_address)
        .call()
        .await?;

    if token_to_permit2_allowance.le(&sell_tokens_u256) {
        let max_approval = U256::from_str(MAX_UINT_HEX)?;
        let approve_result = erc20_contract.approve(permit2_address, max_approval);

        build_requests.push((
            signer.address(),
            token.token,
            100_000,
            approve_result.calldata().clone(),
            U256::from(0),
            nonce,
            "APPROVE".to_string(),
        ));
        nonce += 1;
    }

    // Check Permit2 → UniversalRouter approval
    let permit2_allowance = permit_contract
        .allowance(signer.address(), token.token, universal_router_address)
        .call()
        .await?
        .amount
        .to::<U256>();

    if permit2_allowance.le(&sell_tokens_u256) {
        let approve_result = permit_contract.approve(
            token.token,
            universal_router_address,
            U160::MAX,
            U48::MAX,
        );

        build_requests.push((
            signer.address(),
            permit2_address,
            150_000,
            approve_result.calldata().clone(),
            U256::from(0),
            nonce,
            "APPROVE".to_string(),
        ));
        nonce += 1;
    }

    // === Build V4 buy planner (ETH → Token) using ExactOut ===
    // ExactOut: get exact tokens, spend at most max ETH
    let eth_address = Address::from_str("0x0000000000000000000000000000000000000000")?;
    let buy_exact_tokens_u256 = f64_to_token_units(task.buy_exact_tokens, token.decimals);
    let buy_max_eth_u256 = f64_to_wei(task.buy_max_eth);

    // ExactOut: currency_out = Token (what we want), path from Token back to ETH
    let buy_config = SwapExactOut {
        currency_out: token.token,
        path: encode_multihop_exact_in_path(std::slice::from_ref(&token_pool_key), token.token),
        amount_out: buy_exact_tokens_u256,
        amount_in_maximum: buy_max_eth_u256,
    };

    let mut buy_planner = V4Planner::new();
    buy_planner.add_action(V4Action::SwapExactOut, encode_swap_exact_out(&buy_config));
    // SettleAll: pay ETH (pool determines actual cost, capped by max)
    buy_planner.add_action(
        V4Action::SettleAll,
        encode_settle_all(eth_address, buy_max_eth_u256),
    );
    // TakeAll: receive exact tokens
    buy_planner.add_action(
        V4Action::TakeAll,
        encode_take_all(token.token, buy_exact_tokens_u256),
    );

    // === Build V4 sell planner (Token → ETH) using ExactIn ===
    // Atomic bundle: buy leg impacts pool price before sell executes in the same tx.
    // Discount sell minimum by pool fee + small buffer to account for this.
    let pool_fee_factor = 1.0 - (token_pool_key.fee as f64 / 1_000_000.0);
    let sell_min_eth_u256 = f64_to_wei(task.sell_min_eth * pool_fee_factor * 0.999);

    let sell_config = SwapExactIn {
        currency_in: token.token,
        path: encode_multihop_exact_in_path(std::slice::from_ref(&token_pool_key), token.token),
        amount_in: sell_tokens_u256,
        amount_out_minimum: sell_min_eth_u256,
    };

    let mut sell_planner = V4Planner::new();
    sell_planner.add_action(V4Action::SwapExactIn, encode_swap_exact_in(&sell_config));
    // SettleAll: pay exact tokens
    sell_planner.add_action(
        V4Action::SettleAll,
        encode_settle_all(token.token, sell_tokens_u256),
    );
    // TakeAll: receive ETH (at least min)
    sell_planner.add_action(
        V4Action::TakeAll,
        encode_take_all(eth_address, sell_min_eth_u256),
    );

    // === Combine into a single RoutePlanner ===
    let mut route_planner = RoutePlanner::new();
    route_planner.add_command(CommandType::V4Swap, buy_planner.finalize());
    route_planner.add_command(CommandType::V4Swap, sell_planner.finalize());
    // Sweep excess ETH back to caller (ExactOut buy may not use full buy_max_eth)
    route_planner.add_command(
        CommandType::Sweep,
        encode_sweep(eth_address, signer.address(), U256::from(0)),
    );

    let commands = Bytes::from(route_planner.commands);
    let inputs = route_planner.inputs;

    // tx value = buy_max_eth (ExactOut buy may cost less; SWEEP returns excess)
    build_requests.push((
        signer.address(),
        universal_router_address,
        1_000_000, // 1M gas for two swaps + sweep
        universal_router
            .execute_1(commands, inputs, deadline)
            .calldata()
            .clone(),
        buy_max_eth_u256,
        nonce,
        "BUNDLE_BUY_SELL".to_string(),
    ));

    // === TX watcher for NATS events ===
    let tx_watcher_info = TxWatcherInfo {
        project_id,
        wallet_id: task.wallet_id,
        user_id: task.user_id,
        eth_price: task.eth_price,
        trade_volume_usdc: task.trade_volume_usdc,
    };

    let (tx_sender, mut tx_receiver) = tokio::sync::mpsc::channel::<StreamInfo>(32);
    let nc_clone = nc.clone();
    let user_id = task.user_id;
    let network_name = task.network.clone();
    let db_clone = db.clone();
    tokio::spawn(async move {
        while let Some(stream_info) = tx_receiver.recv().await {
            if let StreamType::TradeConfirmed {
                project_id,
                volume_usdc,
            } = &stream_info.stream_type
            {
                let _ = crate::cache::add_daily_volume(*project_id, *volume_usdc).await;
                // Debounced push — deduplicates buy+sell legs
                crate::requests::wallets::push_financials::schedule_push_wallets_financials(
                    nc_clone.clone(),
                    db_clone.clone(),
                    *project_id,
                    user_id,
                    network_name.clone(),
                );
                continue;
            }
            if let Ok(bytes) = serde_json::to_vec(&stream_info) {
                let _ = nc_clone
                    .publish(subjects::events::evm::TRANSACTION_NEW, bytes.into())
                    .await;
            }
        }
    });

    // === Send all txs (approvals + bundle) ===
    let token_in_buy = network.get_base_token();
    let token_out_buy = token.token;
    let decimals_f64 = u256_to_f64(U256::from(10).pow(U256::from(token.decimals)));

    for (from, to, gas_limit, input, value, req_nonce, tx_type) in build_requests {
        let tx_request = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(value)
            .with_input(input)
            .with_gas_limit(gas_limit.try_into()?)
            .with_max_fee_per_gas(max_fee_per_gas)
            .with_max_priority_fee_per_gas(max_priority_fee_per_gas)
            .with_nonce(req_nonce)
            .with_chain_id(network.clone() as u64);

        let eth_wallet = EthereumWallet::from(signer.clone());
        let tx_envelope = tx_request.build(&eth_wallet).await?;
        let tx: PendingTransactionBuilder<Ethereum> =
            provider.send_tx_envelope(tx_envelope).await?;

        let tx_mpsc = Arc::new(tx_sender.clone());
        let tx_watcher = tx_watcher_info.clone();
        let is_bundle = tx_type == "BUNDLE_BUY_SELL";
        let from_addr = from;
        let token_addr = token_out_buy;
        let base_token = token_in_buy;
        let decimals = decimals_f64;
        let buy_eth_est = task.buy_max_eth;
        let buy_tokens_est = task.buy_exact_tokens;
        let sell_eth_est = task.sell_min_eth;
        let sell_tokens_est = task.sell_tokens;

        tokio::task::spawn(async move {
            if !is_bundle {
                // Approval tx — just wait for confirmation, no volume tracking
                let _ = tx
                    .with_timeout(Some(Duration::from_secs(12)))
                    .get_receipt()
                    .await;
                return;
            }

            let tx_hash = tx.tx_hash().to_string();
            let buy_hash = format!("{}:buy", tx_hash);
            let sell_hash = format!("{}:sell", tx_hash);

            // Record both BUY + SELL transactions atomically (estimated values — updated from receipt below)
            let _ = tx_mpsc
                .send(StreamInfo {
                    user_id: tx_watcher.user_id,
                    stream_type: StreamType::NewTransactionsRequest(vec![
                        NewTransaction {
                            slot: 0,
                            project_id: tx_watcher.project_id,
                            sol_price: f64_to_big_int(tx_watcher.eth_price),
                            value: f64_to_big_int(buy_eth_est),
                            tokens: f64_to_big_int(buy_tokens_est),
                            address: from_addr.to_string(),
                            tx_hash: buy_hash.clone(),
                            tx_type: "BUY".to_string(),
                            token_in: base_token.to_string(),
                            token_out: token_addr.to_string(),
                            date_added: SystemTime::now(),
                        },
                        NewTransaction {
                            slot: 0,
                            project_id: tx_watcher.project_id,
                            sol_price: f64_to_big_int(tx_watcher.eth_price),
                            value: f64_to_big_int(sell_eth_est),
                            tokens: f64_to_big_int(sell_tokens_est),
                            address: from_addr.to_string(),
                            tx_hash: sell_hash.clone(),
                            tx_type: "SELL".to_string(),
                            token_in: token_addr.to_string(),
                            token_out: base_token.to_string(),
                            date_added: SystemTime::now(),
                        },
                    ]),
                })
                .await;

            let receipt = tx
                .with_timeout(Some(Duration::from_secs(12)))
                .get_receipt()
                .await
                .ok();

            if let Some(receipt) = receipt {
                let block = receipt.block_number.unwrap_or(0) as i64;
                let mut actual_buy_eth: Option<f64> = None;
                let mut actual_buy_tokens: Option<f64> = None;
                let mut actual_sell_eth: Option<f64> = None;
                let mut actual_sell_tokens: Option<f64> = None;

                if receipt.status() {
                    // Parse ERC-20 Transfer events for token amounts
                    let transfer_topic = FixedBytes::<32>::from_str(
                        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    )
                    .unwrap();

                    for log in receipt.inner.logs() {
                        if log.inner.address == token_addr
                            && log.inner.data.topics().len() >= 3
                            && log.inner.data.topics()[0] == transfer_topic
                        {
                            let from = Address::from_word(log.inner.data.topics()[1]);
                            let to = Address::from_word(log.inner.data.topics()[2]);
                            let raw = U256::from_be_slice(log.inner.data.data.as_ref());
                            let amount = u256_to_f64(raw) / decimals;

                            if to == from_addr {
                                // Tokens received → buy leg
                                actual_buy_tokens =
                                    Some(actual_buy_tokens.unwrap_or(0.0) + amount);
                            }
                            if from == from_addr {
                                // Tokens sent → sell leg
                                actual_sell_tokens =
                                    Some(actual_sell_tokens.unwrap_or(0.0) + amount);
                            }
                        }
                    }

                    // Parse V4 Swap events for ETH amounts
                    let swap_topic = FixedBytes::<32>::from_str(
                        "0x40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f",
                    )
                    .unwrap();

                    for log in receipt.inner.logs() {
                        if !log.inner.data.topics().is_empty()
                            && log.inner.data.topics()[0] == swap_topic
                            && log.inner.data.data.len() >= 32
                        {
                            let amount0_bytes: [u8; 32] =
                                log.inner.data.data[0..32].try_into().unwrap();

                            if amount0_bytes[0] & 0x80 == 0 {
                                // Positive amount0 = ETH received by swapper (sell leg)
                                let amount0 = U256::from_be_bytes(amount0_bytes);
                                if amount0 > U256::ZERO {
                                    actual_sell_eth = Some(u256_to_f64(amount0) / 1e18);
                                }
                            } else {
                                // Negative amount0 = ETH spent by swapper (buy leg)
                                let abs = (!U256::from_be_bytes(amount0_bytes))
                                    .wrapping_add(U256::from(1));
                                actual_buy_eth = Some(u256_to_f64(abs) / 1e18);
                            }
                        }
                    }
                }

                // Update BUY transaction with actual values from receipt
                let _ = tx_mpsc
                    .send(StreamInfo {
                        user_id: tx_watcher.user_id,
                        stream_type: StreamType::UpdateTransactionSlot {
                            tx_hash: buy_hash,
                            slot: block,
                            value: actual_buy_eth.map(f64_to_big_int),
                            tokens: actual_buy_tokens.map(f64_to_big_int),
                        },
                    })
                    .await;

                // Update SELL transaction with actual values from receipt
                let _ = tx_mpsc
                    .send(StreamInfo {
                        user_id: tx_watcher.user_id,
                        stream_type: StreamType::UpdateTransactionSlot {
                            tx_hash: sell_hash,
                            slot: block,
                            value: actual_sell_eth.map(f64_to_big_int),
                            tokens: actual_sell_tokens.map(f64_to_big_int),
                        },
                    })
                    .await;

                // Track volume for each leg separately (actual from receipt, fallback to estimate)
                if receipt.status() && tx_watcher.trade_volume_usdc > 0.0 {
                    let buy_volume = actual_buy_eth
                        .map(|v| v * tx_watcher.eth_price)
                        .unwrap_or(tx_watcher.trade_volume_usdc);

                    let _ = tx_mpsc
                        .send(StreamInfo {
                            user_id: tx_watcher.user_id,
                            stream_type: StreamType::TradeConfirmed {
                                project_id: tx_watcher.project_id,
                                volume_usdc: buy_volume,
                            },
                        })
                        .await;

                    let sell_volume = actual_sell_eth
                        .map(|v| v * tx_watcher.eth_price)
                        .unwrap_or(tx_watcher.trade_volume_usdc);

                    let _ = tx_mpsc
                        .send(StreamInfo {
                            user_id: tx_watcher.user_id,
                            stream_type: StreamType::TradeConfirmed {
                                project_id: tx_watcher.project_id,
                                volume_usdc: sell_volume,
                            },
                        })
                        .await;
                }
            }
        });
    }

    let _ = crate::cache::reset_task_failures(project_id).await;
    Ok(())
}

/// V2/V3 sequential fallback: reuse existing buy/sell handlers
async fn process_v2v3_sequential(
    nc: async_nats::Client,
    db: Database,
    task: EvmProcessBundleBuySell,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing::info!(
        "[EVM] Bundle V2/V3 sequential: project={} wallet={} buy={:.6} ETH sell={:.4} tokens",
        task.project_id,
        task.address,
        task.buy_max_eth,
        task.sell_tokens,
    );

    // V2/V3 buy: ExactIn with max_eth as value, expect at least buy_exact_tokens
    let buy_task = EvmProcessBuy {
        project_id: task.project_id,
        wallet_id: task.wallet_id,
        user_id: task.user_id,
        token: task.token.clone(),
        pool: task.pool.clone(),
        value: task.buy_max_eth,
        tokens: task.buy_exact_tokens,
        private_key: task.private_key.clone(),
        address: task.address.clone(),
        eth_price: task.eth_price,
        network: task.network.clone(),
        token_info: task.token_info.clone(),
        pool_info: task.pool_info.clone(),
        trade_volume_usdc: task.trade_volume_usdc,
    };

    // V2/V3 sell: ExactIn with sell_tokens, expect at least sell_min_eth back
    let sell_task = EvmProcessSell {
        project_id: task.project_id,
        wallet_id: task.wallet_id,
        user_id: task.user_id,
        token: task.token,
        pool: task.pool,
        value: task.sell_min_eth,
        tokens: task.sell_tokens,
        private_key: task.private_key,
        address: task.address,
        eth_price: task.eth_price,
        network: task.network,
        token_info: task.token_info,
        pool_info: task.pool_info,
        trade_volume_usdc: task.trade_volume_usdc,
    };

    // Sequential: buy first, then sell
    if let Err(e) = process_buy(nc.clone(), db.clone(), buy_task).await {
        tracing::error!("[EVM] Bundle buy leg failed: {}", e);
    }
    if let Err(e) = process_sell(nc, db, sell_task).await {
        tracing::error!("[EVM] Bundle sell leg failed: {}", e);
    }

    Ok(())
}
