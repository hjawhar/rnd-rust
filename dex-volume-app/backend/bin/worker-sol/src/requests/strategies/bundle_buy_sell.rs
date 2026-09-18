use vm_data::models::streams::{StreamInfo, StreamType};
use vm_data::models::task::ProcessBundleBuySell;
use vm_data::models::transaction::NewTransaction;
use vm_data::utils::helpers::f64_to_big_int;
use vm_solana::{
    markets::generic::market_pair::{MarketEnum, MarketPair},
    markets::traits::{SwapArgs, SwapDirection},
    utils::{
        helpers::{send_jito_bundle_multi, simulate_jito_bundle},
        market_impact::{calculate_buy_impact, calculate_sell_impact, validate_impact},
    },
};
use solana_sdk::message::v0::Message as V0Message;
use solana_sdk::message::VersionedMessage;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::transaction::VersionedTransaction;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;

use crate::cache::{
    price_cache::add_tx_sol_price_at_time, volume_cache::set_trade_volume_for_tx,
};
use crate::requests::swap_orchestrator::SwapOrchestrator;
use crate::state::get_state;
use vm_solana::utils::constants::JITO_TIP_ACCOUNT;
use vm_solana::utils::helpers::mul_f64_and_u64_to_u64;

pub async fn process_bundle_buy_sell(task: ProcessBundleBuySell, nc: async_nats::Client) {
    let state = get_state();
    let market_pair: MarketPair = match serde_json::from_str(&task.market_pair) {
        Ok(mp) => mp,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to deserialize market_pair: {}", e);
            return;
        }
    };

    let tip = task.jito_tip;
    let rpc_client = state.rpc.clone();

    // Parse both keypairs
    let buy_kp = match vm_solana::utils::helpers::str_to_pk(task.buy_private_key.clone()) {
        Ok(kp) => kp,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to parse buy private key: {}", e);
            return;
        }
    };
    let sell_kp = match vm_solana::utils::helpers::str_to_pk(task.sell_private_key.clone()) {
        Ok(kp) => kp,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to parse sell private key: {}", e);
            return;
        }
    };

    // Market impact validation for both sides (if configured)
    if let Some(max_impact_bps) = task.max_market_impact_bps {
        let market = market_pair.to_market();

        // Buy side: check quote vault (SOL side)
        let quote_vault_balance = match market.get_quote_vault() {
            Ok(vault) => {
                state.get_vault_balance(&vault.to_string()).map(|b| b as f64 / 1e9)
            }
            Err(_) => None,
        };

        if let Ok(impact) = calculate_buy_impact(&market_pair, task.buy_value, quote_vault_balance)
            && let Err(e) = validate_impact(&impact, max_impact_bps) {
                tracing::error!("[BUNDLE] Buy side: {}", e);
                let msg = format!(
                    "Bundle skipped for project {} - buy side: {}. Retrying next interval.",
                    task.project_id, e
                );
                let _ = nc
                    .publish(vm_nats::subjects::discord::MESSAGE, msg.into())
                    .await;
                return;
            }

        // Sell side: check base vault (token side)
        let base_vault_balance = match market.get_base_vault() {
            Ok(vault) => {
                state.get_vault_balance(&vault.to_string()).map(|b| b as f64)
            }
            Err(_) => None,
        };

        if let Ok(impact) =
            calculate_sell_impact(&market_pair, task.sell_tokens, base_vault_balance)
            && let Err(e) = validate_impact(&impact, max_impact_bps) {
                tracing::error!("[BUNDLE] Sell side: {}", e);
                let msg = format!(
                    "Bundle skipped for project {} - sell side: {}. Retrying next interval.",
                    task.project_id, e
                );
                let _ = nc
                    .publish(vm_nats::subjects::discord::MESSAGE, msg.into())
                    .await;
                return;
            }
    }

    // Bundle is atomic — slippage between buy and sell is impossible.
    // DEXes with exact output: buy exactly N tokens, sell exactly N tokens.
    // DEXes without (Meteora): buy with exact SOL, sell estimated tokens.
    // RPC simulation pre-check catches any mismatch before submission.
    let supports_exact_output = matches!(
        market_pair.market,
        MarketEnum::RaydiumAMMV4(_) | MarketEnum::RaydiumCLMM(_) | MarketEnum::PumpfunAMM(_)
    );

    let (buy_args, sell_args) = if supports_exact_output {
        // Exact output: buy exactly sell_tokens, max SOL = 1.2x estimate (enough headroom,
        // simulation catches real issues; 2x was too aggressive and caused insufficient lamports)
        let buy = SwapArgs::exact_output(
            task.sell_tokens as u64,
            mul_f64_and_u64_to_u64(task.buy_value * 1.2, LAMPORTS_PER_SOL),
        );
        // Sell all tokens, min SOL = 1 lamport (atomic bundle, no slippage risk)
        let sell = SwapArgs::new(task.sell_tokens as u64, 1);
        (buy, sell)
    } else {
        // Meteora: exact input buy (spend exact SOL), min tokens = 1 (simulation is the guard)
        let buy = SwapArgs::new(
            mul_f64_and_u64_to_u64(task.buy_value, LAMPORTS_PER_SOL),
            1,
        );
        // Sell fee-adjusted tokens — tokens_out estimate doesn't account for pool fee,
        // so buy receives ~fee% fewer tokens than estimated. Reduce sell to match.
        let fee_bps = market_pair.to_market().metadata()
            .map(|m| m.fees.trade_fee_bps).unwrap_or(0);
        let fee_factor = 1.0 - (fee_bps as f64 / 10000.0);
        let sell_tokens = (task.sell_tokens * fee_factor) as u64;
        let sell = SwapArgs::new(sell_tokens, 1);
        (buy, sell)
    };

    // Parallel instruction building via tokio::join!
    let orchestrator = SwapOrchestrator::new(rpc_client.clone());
    let (buy_result, sell_result) = tokio::join!(
        orchestrator.build_swap(&market_pair, buy_kp.pubkey(), buy_args, SwapDirection::Buy),
        orchestrator.build_swap(&market_pair, sell_kp.pubkey(), sell_args, SwapDirection::Sell),
    );

    let buy_ixs = match buy_result {
        Ok(ixs) => ixs,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to build buy instructions: {}", e);
            return;
        }
    };
    let mut sell_ixs = match sell_result {
        Ok(ixs) => ixs,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to build sell instructions: {}", e);
            return;
        }
    };

    // Add Jito tip only on sell tx (last in bundle) — saves tip on buy tx
    if tip > 0.0 {
        sell_ixs.push(system_instruction::transfer(
            &sell_kp.pubkey(),
            &solana_pubkey::Pubkey::from_str_const(JITO_TIP_ACCOUNT),
            mul_f64_and_u64_to_u64(tip, LAMPORTS_PER_SOL),
        ));
    }

    // Fetch blockhash once, shared for both transactions
    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(bh) => bh,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to fetch latest blockhash: {}", e);
            return;
        }
    };

    // Compile and sign buy tx
    let buy_msg = match V0Message::try_compile(&buy_kp.pubkey(), &buy_ixs, &[], blockhash) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to compile buy message: {}", e);
            return;
        }
    };
    let buy_tx = match VersionedTransaction::try_new(VersionedMessage::V0(buy_msg), &[&buy_kp]) {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to create buy transaction: {}", e);
            return;
        }
    };

    // Compile and sign sell tx
    let sell_msg = match V0Message::try_compile(&sell_kp.pubkey(), &sell_ixs, &[], blockhash) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("[BUNDLE] Failed to compile sell message: {}", e);
            return;
        }
    };
    let sell_tx =
        match VersionedTransaction::try_new(VersionedMessage::V0(sell_msg), &[&sell_kp]) {
            Ok(tx) => tx,
            Err(e) => {
                tracing::error!("[BUNDLE] Failed to create sell transaction: {}", e);
                return;
            }
        };

    // Pre-flight RPC simulation for buy tx — catches account/balance issues early
    match rpc_client.simulate_transaction(&buy_tx).await {
        Ok(result) => {
            if let Some(err) = result.value.err {
                let logs = result.value.logs.unwrap_or_default().join("\n  ");
                tracing::error!(
                    "[BUNDLE] RPC simulation FAILED for buy tx, project {}:\n  error: {:?}\n  logs:\n  {}",
                    task.project_id, err, logs
                );
                return;
            }
        }
        Err(e) => {
            tracing::error!("[BUNDLE] RPC simulate_transaction failed for project {}: {}", task.project_id, e);
            return;
        }
    }

    // Pre-flight Jito bundle simulation — skip if either tx would fail on-chain
    if let Err(e) = simulate_jito_bundle(&[buy_tx.clone(), sell_tx.clone()], &["BUY", "SELL"]).await {
        tracing::error!("[BUNDLE] Jito simulation FAILED for project {}:\n{}", task.project_id, e);
        return;
    }
    tracing::info!("[BUNDLE] Jito simulation OK for project {}", task.project_id);

    // Track volume for both signatures
    let buy_sig = bs58::encode(buy_tx.signatures[0]).into_string();
    let sell_sig = bs58::encode(sell_tx.signatures[0]).into_string();

    let _ = add_tx_sol_price_at_time(buy_sig.clone(), task.sol_price).await;
    let _ = add_tx_sol_price_at_time(sell_sig.clone(), task.sol_price).await;
    if task.trade_volume_usdc > 0.0 {
        let _ = set_trade_volume_for_tx(&buy_sig, task.trade_volume_usdc).await;
        let _ = set_trade_volume_for_tx(&sell_sig, task.trade_volume_usdc).await;
    }

    tracing::info!(
        "[BUNDLE] Project {} | wallet={} | exact_output={} | buy={:.4} SOL min_tokens={:.0} | sell_tokens={:.0} min_sol={:.4} | tip={:.5} | ${:.2}/leg",
        task.project_id, task.buy_address, supports_exact_output,
        task.buy_value, task.buy_min_tokens,
        task.sell_tokens, task.sell_min_value,
        task.jito_tip, task.trade_volume_usdc,
    );

    // Send as single atomic Jito bundle (buy first, then sell)
    match send_jito_bundle_multi(vec![buy_tx, sell_tx]).await {
        Ok(bundle_id) => {
            tracing::info!(
                "[BUNDLE] Project {} | submitted bundle={} | buy_sig={} sell_sig={}",
                task.project_id, bundle_id, buy_sig, sell_sig,
            );

            // Pre-insert both txs atomically via NATS batch — Geyser re-confirmation
            // will hit on_conflict(tx_hash).do_nothing() → no duplicate WS broadcast.
            // UpdateTransactionSlot from Geyser fills in real slot/value/tokens.
            let decimals = 10_u64.pow(task.token_decimals as u32) as f64;
            let wrapper = StreamInfo {
                user_id: -1,
                stream_type: StreamType::NewTransactionsRequest(vec![
                    NewTransaction {
                        slot: 0,
                        project_id: task.project_id,
                        sol_price: f64_to_big_int(task.sol_price),
                        value: f64_to_big_int(task.buy_value),
                        tokens: f64_to_big_int(task.buy_min_tokens / decimals),
                        address: task.buy_address.clone(),
                        tx_hash: buy_sig.clone(),
                        tx_type: "BUY".to_string(),
                        token_in: "So11111111111111111111111111111111111111112".to_string(),
                        token_out: task.token.clone(),
                        date_added: std::time::SystemTime::now(),
                    },
                    NewTransaction {
                        slot: 0,
                        project_id: task.project_id,
                        sol_price: f64_to_big_int(task.sol_price),
                        value: f64_to_big_int(task.sell_min_value),
                        tokens: f64_to_big_int(task.sell_tokens / decimals),
                        address: task.sell_address.clone(),
                        tx_hash: sell_sig.clone(),
                        tx_type: "SELL".to_string(),
                        token_in: task.token.clone(),
                        token_out: "So11111111111111111111111111111111111111112".to_string(),
                        date_added: std::time::SystemTime::now(),
                    },
                ]),
            };
            if let Ok(bytes) = serde_json::to_vec(&wrapper) {
                let _ = nc.publish(vm_nats::subjects::events::sol::TRANSACTION_NEW, bytes.into()).await;
            }
        }
        Err(e) => {
            tracing::error!("[BUNDLE] Jito bundle failed for project {}: {}", task.project_id, e);
        }
    }
}
