use crate::cache::{
    price_cache::get_tx_sol_price_at_time,
    volume_cache::{add_daily_volume, take_trade_volume_for_tx},
};
use crate::requests::wallets::push_financials::schedule_push_wallets_financials;
use crate::state::get_state;
use borsh::de::BorshDeserialize;
use vm_data::db::Database;
use vm_data::models::{
    streams::{DailyVolumeInfo, StreamInfo, StreamType},
    transaction::NewTransaction,
    wallet_relation::BalanceUpdate,
};
use vm_data::utils::helpers::f64_to_big_int;
use vm_nats::subjects;
use vm_solana::markets::{
    generic::market_pair::{MarketEnum, MarketPair},
    raydium_clmm::models::raydium_clmm_pool::RaydiumCLMMPool,
};
use vm_solana::rpc::fetch_tx_data::get_tx_data;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_sdk::program_pack::Pack;
use spl_token::state::Account;
use std::time::SystemTime;
use yellowstone_grpc_proto::geyser::{SubscribeUpdateAccount, SubscribeUpdateTransaction};

/// Process a Geyser account update directly (no protobuf decode — already decoded).
///
/// Handles: balance tracking, CLMM pair updates, BalanceUpdate broadcasts.
pub async fn process_account_update(
    nc: async_nats::Client,
    _db: Database,
    account_update: SubscribeUpdateAccount,
) {
    let state = get_state();
    let Some(account) = account_update.account else {
        return;
    };

    let acc_key = bs58::encode(&account.pubkey).into_string();
    if !state.is_tracked(&acc_key) {
        return;
    }

    if account.data.is_empty() {
        let balance = account.lamports as f64 / LAMPORTS_PER_SOL as f64;
        state.set_balance(acc_key.clone(), balance);
    } else {
        let found_pair = state.get_pair(&acc_key);
        if let Some(found_pair) = found_pair {
            if let MarketEnum::RaydiumCLMM(_) = found_pair.market {
                let deserialized = RaydiumCLMMPool::deserialize(&mut &account.data[8..]);
                if let Ok(deserialized) = deserialized {
                    let market_pair = MarketPair {
                        name: found_pair.name,
                        pair: found_pair.pair,
                        market: MarketEnum::RaydiumCLMM(deserialized),
                    };
                    state.set_pair(acc_key.clone(), market_pair);
                }
            }
        } else {
            // Token-2022 accounts have extension data beyond the base 165-byte layout.
            // Slice to Account::LEN to handle both standard and Token-2022 accounts.
            let data = &account.data;
            if data.len() >= Account::LEN {
                let account_parsed = Account::unpack(&data[..Account::LEN]);
                if let Ok(account_parsed) = account_parsed {
                    state.set_balance(acc_key.clone(), account_parsed.amount as f64);
                }
            }
        }
    }

    let balance = state.get_balance(&acc_key);
    let relations = state.get_flat_tracked_addresses();
    if let Some(balance) = balance {
        let correct_relations: Vec<_> =
            relations.iter().filter(|x| x.address == acc_key).collect();
        for relation in correct_relations {
            let balance_update = BalanceUpdate {
                balance,
                relation: relation.clone(),
            };
            let event = StreamType::BalanceUpdate(balance_update);
            let wrapper = StreamInfo {
                user_id: relation.user_id,
                stream_type: event,
            };
            let bytes = serde_json::to_vec(&wrapper).unwrap_or_default();
            let _ = nc
                .publish(subjects::events::sol::BALANCE_UPDATE, bytes.into())
                .await;
        }
    }
}

/// Process a Geyser transaction update directly (no protobuf decode — already decoded).
///
/// Handles: tx analysis, NewTransaction broadcast, daily volume tracking, push financials.
pub async fn process_transaction_update(
    nc: async_nats::Client,
    db: Database,
    tx_update: SubscribeUpdateTransaction,
) {
    let state = get_state();
    let t0 = std::time::Instant::now();

    let Some(ref transaction) = tx_update.transaction else {
        return;
    };
    let Some(ref transaction) = transaction.transaction else {
        return;
    };
    let Some(ref message) = transaction.message else {
        return;
    };

    let acc_key = bs58::encode(&message.account_keys[0]).into_string();
    if !state.is_tracked(&acc_key) {
        return;
    }

    let analysis = get_tx_data(tx_update);
    let Ok(analysis) = analysis else { return };
    let Some(analysis) = analysis else { return };

    let found = state.get_wallet(&analysis.address);
    let sol_price = get_tx_sol_price_at_time(analysis.tx_hash.clone()).await;

    let Some(found) = found else { return };
    let Ok(Some(sol_price)) = sol_price else { return };

    let project_id = found.wallet.project_id;
    let tx_hash = analysis.tx_hash.clone();

    tracing::debug!(
        "[GEYSER] Tx confirmed | project={} | tx={} | type={} | slot={} | time={} | geyser_process={}ms",
        project_id,
        tx_hash,
        analysis.tx_type,
        analysis.slot,
        chrono::Utc::now().format("%H:%M:%S%.3f"),
        t0.elapsed().as_millis(),
    );

    let new_tx = NewTransaction {
        slot: analysis.slot as i64,
        project_id,
        value: f64_to_big_int(analysis.value),
        tokens: f64_to_big_int(analysis.tokens),
        address: analysis.address.clone(),
        tx_hash: analysis.tx_hash,
        tx_type: analysis.tx_type,
        token_in: analysis.token_in,
        token_out: analysis.token_out,
        date_added: SystemTime::now(),
        sol_price: f64_to_big_int(sol_price),
    };

    let wrapper = StreamInfo {
        user_id: found.user_id,
        stream_type: StreamType::NewTransactionRequest(new_tx),
    };
    let bytes = serde_json::to_vec(&wrapper).unwrap_or_default();
    let _ = nc
        .publish(subjects::events::sol::TRANSACTION_NEW, bytes.into())
        .await;

    // Track daily volume on confirmed tx
    if let Ok(Some(trade_usdc)) = take_trade_volume_for_tx(&tx_hash).await
        && let Ok(total) = add_daily_volume(project_id, trade_usdc).await
    {
        let target = state.get_task(project_id)
            .map(|t| {
                vm_data::utils::helpers::big_int_to_f64(
                    t.project.trading_daily_volume.clone(),
                )
            })
            .unwrap_or(0.0);
        let volume_info = DailyVolumeInfo {
            project_id,
            volume_usdc: total,
            target_usdc: target,
        };
        let broadcast = StreamInfo {
            user_id: found.user_id,
            stream_type: StreamType::DailyVolumeUpdate(volume_info),
        };
        let bytes = serde_json::to_vec(&broadcast).unwrap_or_default();
        let _ = nc
            .publish(subjects::events::sol::DAILY_VOLUME, bytes.into())
            .await;
    }

    tracing::trace!(
        "[GEYSER] Tx processed | project={} | tx={} | total={}ms",
        project_id,
        tx_hash,
        t0.elapsed().as_millis(),
    );

    // Debounced push
    schedule_push_wallets_financials(nc, db, project_id, found.user_id);
}
