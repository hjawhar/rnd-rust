use vm_data::models::task::ProcessSell;
use vm_solana::{
    markets::generic::market_pair::MarketPair,
    markets::traits::{SwapArgs, SwapDirection},
    utils::{
        helpers::{send_jito_bundle, simulate_jito_bundle},
        market_impact::{calculate_sell_impact, validate_impact},
    },
};
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message as V0Message;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_system_interface::instruction as system_instruction;
use solana_sdk::transaction::VersionedTransaction;
use solana_signer::Signer;
use spl_associated_token_account::get_associated_token_address_with_program_id;

use crate::cache::{
    price_cache::add_tx_sol_price_at_time,
    volume_cache::set_trade_volume_for_tx,
};
use crate::requests::swap_orchestrator::SwapOrchestrator;
use crate::state::get_state;
use vm_solana::utils::constants::JITO_TIP_ACCOUNT;
use vm_solana::utils::helpers::mul_f64_and_u64_to_u64;

pub async fn process_sell(task: ProcessSell, nc: async_nats::Client) {
    let state = get_state();
    let market_pair: MarketPair = match serde_json::from_str(&task.market_pair) {
        Ok(mp) => mp,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to deserialize market_pair: {}", e);
            return;
        }
    };

    let tip = task.jito_tip;
    let rpc_client = state.rpc.clone();

    // Phase 1.2: Pre-warm ATA cache for source and destination token accounts
    let user_address = match vm_solana::utils::helpers::str_to_pk(task.private_key.clone()) {
        Ok(kp) => kp.pubkey(),
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to parse private key: {}", e);
            return;
        }
    };

    // Phase 4: Get token mint using Market trait (works for all DEX types!)
    let market = market_pair.to_market();
    // For SELL: Token → SOL, so we're selling the quote mint (the token)
    let token_mint = match market.get_quote_mint() {
        Ok(mint) => mint,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to get quote mint: {}", e);
            return;
        }
    };

    // Pre-warm source ATA cache (detect Token vs Token-2022 for correct ATA)
    let token_program = state.fetch_token_program(&token_mint.to_string()).await
        .unwrap_or(spl_token::ID);
    let source_ata = get_associated_token_address_with_program_id(&user_address, &token_mint, &token_program);
    let _ = state.check_ata_existence(&[source_ata]).await;

    // Pre-warm token metadata cache
    let _ = state.fetch_token_info(&token_mint.to_string()).await;

    // Pre-warm base vault balance cache (token side for sells)
    let base_vault_balance = match market.get_base_vault() {
        Ok(vault) => {
            state.get_vault_balance(&vault.to_string()).map(|b| b as f64)
        }
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to get base vault: {}", e);
            None
        }
    };

    // Market impact validation (if configured)
    if let Some(max_impact_bps) = task.max_market_impact_bps {
        match calculate_sell_impact(&market_pair, task.tokens, base_vault_balance) {
            Ok(impact_result) => {
                if let Err(e) = validate_impact(&impact_result, max_impact_bps) {
                    tracing::error!("[PROCESS_SELL] {}", e);

                    let msg = format!(
                        "Sell skipped for project {} - {}. Retrying next interval.",
                        task.project_id, e
                    );
                    let _ = nc
                        .publish(
                            vm_nats::subjects::discord::MESSAGE,
                            msg.into(),
                        )
                        .await;

                    return;
                }
            }
            Err(e) => {
                tracing::error!("[PROCESS_SELL] Failed to calculate market impact: {}", e);
            }
        }
    }

    // Phase 3: Use SwapOrchestrator for pure instruction building (200x faster!)
    let kp = match vm_solana::utils::helpers::str_to_pk(task.private_key.clone()) {
        Ok(kp) => kp,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to parse private key: {}", e);
            return;
        }
    };

    // Build swap arguments
    let swap_args = SwapArgs::new(
        task.tokens as u64,
        mul_f64_and_u64_to_u64(task.value, LAMPORTS_PER_SOL),
    );

    // Use SwapOrchestrator for optimized instruction building
    let orchestrator = SwapOrchestrator::new(rpc_client.clone());
    let swap_result = orchestrator
        .build_swap(
            &market_pair,
            kp.pubkey(),
            swap_args,
            SwapDirection::Sell,
        )
        .await;

    let mut ixs = match swap_result {
        Ok(ixs) => ixs,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to build swap instructions: {}", e);
            return;
        }
    };

    // Add Jito tip
    if tip > 0.0 {
        ixs.push(system_instruction::transfer(
            &kp.pubkey(),
            &solana_pubkey::Pubkey::from_str_const(JITO_TIP_ACCOUNT),
            mul_f64_and_u64_to_u64(tip, LAMPORTS_PER_SOL),
        ));
    }

    // Build and sign transaction
    let blockhash = match rpc_client.get_latest_blockhash().await {
        Ok(blockhash) => blockhash,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to fetch latest blockhash: {}", e);
            return;
        }
    };

    let message = match V0Message::try_compile(&kp.pubkey(), &ixs, &[], blockhash) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to compile message: {}", e);
            return;
        }
    };

    let tx = match VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp]) {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("[PROCESS_SELL] Failed to create transaction: {}", e);
            return;
        }
    };

    // Pre-flight Jito bundle simulation — skip if tx would fail on-chain
    if let Err(e) = simulate_jito_bundle(std::slice::from_ref(&tx), &["SELL"]).await {
        tracing::error!("[PROCESS_SELL] Jito simulation FAILED for project {}:\n{}", task.project_id, e);
        return;
    }
    tracing::info!("[PROCESS_SELL] Jito simulation OK for project {}", task.project_id);

    let signature = bs58::encode(tx.signatures[0]).into_string();

    let _ = add_tx_sol_price_at_time(signature.clone(), task.sol_price).await;
    if task.trade_volume_usdc > 0.0 {
        let _ = set_trade_volume_for_tx(&signature, task.trade_volume_usdc).await;
    }

    let jito_response = send_jito_bundle(tx).await;
    if let Err(e) = &jito_response {
        tracing::error!("[PROCESS_SELL] Jito bundle failed: {}", e);
    }

}
