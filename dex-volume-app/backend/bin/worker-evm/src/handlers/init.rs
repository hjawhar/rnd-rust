use alloy::primitives::{Address, B256, FixedBytes, U256};
use alloy::providers::{Provider, RootProvider};
use alloy::rpc::types::TransactionReceipt;
use bigdecimal::ToPrimitive;
use vm_data::db::Database;
use vm_data::models::streams::StreamType;
use vm_data::models::transaction::Transaction;
use vm_evm::constants::Network;
use vm_evm::contracts::ERC20Contract;
use vm_data::utils::helpers::f64_to_big_int;
use vm_evm::helpers::u256_to_f64;
use vm_nats::subjects;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;

/// Receipt fetch chunk size — fire 50 concurrently, then wait 1s before the next chunk.
const RECEIPT_CHUNK_SIZE: usize = 50;

/// Self-initialize worker-evm by recovering orphaned ownership, resolving pending
/// transactions, and requesting init data from api-server. Cache population is
/// deferred to TASK_START — only data-integrity operations run here.
/// Non-fatal on failure — worker can still receive runtime commands.
pub async fn self_initialize(nc: &async_nats::Client, db: &Database) {
    let worker_id = crate::get_worker_id();
    tracing::info!("[EVM] Requesting init data from api-server...");

    // Recover ownership from a prior crash: release any projects this worker
    // previously owned whose heartbeat has expired.
    match vm_redis::task_ownership::get_owned_projects("evm", worker_id).await {
        Ok(project_ids) => {
            for pid in &project_ids {
                match vm_redis::task_heartbeat::is_alive("evm", *pid).await {
                    Ok(false) => {
                        if let Err(e) =
                            vm_redis::task_ownership::force_release("evm", *pid).await
                        {
                            tracing::warn!(
                                "[EVM][INIT] Failed to force-release project {}: {}",
                                pid, e
                            );
                        } else {
                            tracing::info!(
                                "[EVM][INIT] Released orphaned project {} (heartbeat dead)",
                                pid
                            );
                        }
                    }
                    Ok(true) => {
                        tracing::warn!(
                            "[EVM][INIT] Project {} still has live heartbeat after restart",
                            pid
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[EVM][INIT] Failed to check heartbeat for project {}: {}",
                            pid, e
                        );
                    }
                }
            }
            if !project_ids.is_empty() {
                tracing::info!(
                    "[EVM][INIT] Ownership recovery complete, checked {} project(s)",
                    project_ids.len()
                );
            }
        }
        Err(e) => {
            tracing::warn!("[EVM][INIT] Failed to query owned projects: {}", e);
        }
    }

    // Resolve unconfirmed EVM transactions (slot=0) from previous runs (non-blocking)
    let db_clone = db.clone();
    tokio::spawn(async move {
        resolve_pending_transactions(&db_clone).await;
    });

    let request = StreamType::RequestInitData;
    let bytes = match serde_json::to_vec(&request) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("[EVM] Failed to serialize init request: {}", e);
            return;
        }
    };

    let response = vm_nats::request_with_timeout(
        nc,
        subjects::rpc::evm::INIT_DATA,
        bytes,
        Duration::from_secs(15),
    )
    .await;

    match response {
        Ok(msg) => {
            if let Ok(StreamType::ResponseInitData(payload)) =
                serde_json::from_slice::<StreamType>(&msg.payload)
            {
                // Cache population (track_address, wallets) is deferred to TASK_START
                // handlers — each project's state is populated when ownership is claimed.
                tracing::info!(
                    "[EVM] Self-initialization complete: {} projects, {} wallets available (cache deferred to TASK_START)",
                    payload.projects.len(),
                    payload.wallets_projects.len()
                );
            } else {
                tracing::warn!("[EVM] Unexpected init data response format");
            }
        }
        Err(e) => {
            tracing::warn!("[EVM] Self-initialization failed (non-fatal): {}", e);
        }
    }
}

/// Resolve unconfirmed EVM transactions (slot=0) by checking receipts on-chain.
/// - If receipt found: parse actual values from logs, update DB with block number + real amounts
/// - If no receipt: delete the transaction from DB
///
/// Groups by network (one provider per chain), fires all receipt checks concurrently.
async fn resolve_pending_transactions(db: &Database) {
    let pending = match db.get_unconfirmed_evm_transactions().await {
        Ok(txs) => txs,
        Err(e) => {
            tracing::warn!("[EVM] Failed to fetch unconfirmed transactions: {}", e);
            return;
        }
    };

    if pending.is_empty() {
        return;
    }

    tracing::info!(
        "[EVM] Resolving {} unconfirmed transaction(s)...",
        pending.len()
    );

    // Build one provider per network
    let mut providers: HashMap<String, RootProvider> = HashMap::new();

    // Pre-filter: parse hashes and resolve providers, delete invalid entries immediately
    struct ValidTx {
        tx: Transaction,
        hash: B256,
        network_name: String,
    }

    let mut valid: Vec<ValidTx> = Vec::with_capacity(pending.len());
    let mut deleted = 0u32;

    for (tx, network_name) in pending {
        let Some(network) = Network::from_network_name(&network_name) else {
            tracing::warn!(
                "[EVM] Unknown network '{}' for tx id={}, deleting",
                network_name,
                tx.id
            );
            let _ = db.delete_transaction(tx.id).await;
            deleted += 1;
            continue;
        };

        let hash = match tx.tx_hash.parse::<B256>() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    "[EVM] Invalid tx hash '{}' for tx id={}: {}, deleting",
                    tx.tx_hash,
                    tx.id,
                    e
                );
                let _ = db.delete_transaction(tx.id).await;
                deleted += 1;
                continue;
            }
        };

        providers
            .entry(network_name.clone())
            .or_insert_with(|| network.get_provider());

        valid.push(ValidTx {
            tx,
            hash,
            network_name,
        });
    }

    if valid.is_empty() {
        if deleted > 0 {
            tracing::info!(
                "[EVM] Resolved pending txs: 0 confirmed, {} deleted, 0 errors",
                deleted
            );
        }
        return;
    }

    // ── Phase 1: Fetch receipts in chunks of 50/s ─────────────────────
    let tx_map: HashMap<i32, &ValidTx> = valid.iter().map(|v| (v.tx.id, v)).collect();

    struct ConfirmedTx<'a> {
        entry: &'a ValidTx,
        receipt: TransactionReceipt,
    }

    let mut confirmed_txs: Vec<ConfirmedTx> = Vec::new();
    let mut errors = 0u32;

    let total_chunks = valid.len().div_ceil(RECEIPT_CHUNK_SIZE);

    for (chunk_idx, chunk) in valid.chunks(RECEIPT_CHUNK_SIZE).enumerate() {
        if chunk_idx > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        tracing::info!(
            "[EVM] Fetching receipts chunk {}/{} ({} txs)",
            chunk_idx + 1,
            total_chunks,
            chunk.len()
        );

        let results: Vec<_> = futures::future::join_all(chunk.iter().map(|entry| {
            let provider = providers[&entry.network_name].clone();
            let hash = entry.hash;
            let tx_id = entry.tx.id;
            async move {
                let result = provider.get_transaction_receipt(hash).await;
                (tx_id, result)
            }
        }))
        .await;

        for (tx_id, result) in results {
            let entry = tx_map[&tx_id];

            match result {
                Ok(Some(receipt)) => {
                    tracing::debug!(
                        "[EVM] Receipt found for {} on {} (block {}, status={})",
                        entry.tx.tx_hash,
                        entry.network_name,
                        receipt.block_number.unwrap_or(0),
                        if receipt.status() { "success" } else { "reverted" }
                    );
                    confirmed_txs.push(ConfirmedTx { entry, receipt });
                }
                Ok(None) => {
                    tracing::debug!(
                        "[EVM] No receipt for {} on {}, deleting",
                        entry.tx.tx_hash,
                        entry.network_name
                    );
                    if let Err(e) = db.delete_transaction(tx_id).await {
                        tracing::warn!(
                            "[EVM] Failed to delete unconfirmed tx {}: {}",
                            entry.tx.tx_hash,
                            e
                        );
                        errors += 1;
                    } else {
                        deleted += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[EVM] Failed to fetch receipt for {} on {}: {}",
                        entry.tx.tx_hash,
                        entry.network_name,
                        e
                    );
                    errors += 1;
                }
            }
        }
    }

    // ── Phase 2: Fetch token decimals for BUY trades (batch per token) ──
    // BUY tx_type: token_out is the token we received — need decimals to parse Transfer logs
    let mut decimals_needed: HashSet<(String, String)> = HashSet::new(); // (token_out, network_name)
    for ct in &confirmed_txs {
        if ct.entry.tx.tx_type == "BUY" && ct.receipt.status() {
            decimals_needed.insert((
                ct.entry.tx.token_out.clone(),
                ct.entry.network_name.clone(),
            ));
        }
    }

    let mut decimals_map: HashMap<String, f64> = HashMap::new(); // token_address → 10^decimals

    if !decimals_needed.is_empty() {
        let results = futures::future::join_all(
            decimals_needed.iter().filter_map(|(token_addr, network_name)| {
                let provider = providers[network_name].clone();
                let addr_str = token_addr.clone();
                let addr = addr_str.parse::<Address>().ok()?;
                Some(async move {
                    let erc20 = ERC20Contract::new(addr, &provider);
                    let result = erc20.decimals().call().await;
                    (addr_str, result)
                })
            }),
        )
        .await;

        for (addr, result) in results {
            match result {
                Ok(dec) => {
                    let d: u32 = dec.try_into().unwrap_or(18);
                    decimals_map.insert(addr, 10f64.powi(d as i32));
                }
                Err(e) => {
                    tracing::warn!("[EVM] Failed to fetch decimals for {}: {}", addr, e);
                }
            }
        }
    }

    // ── Phase 3: Parse receipts and update DB ───────────────────────────
    // ERC-20 Transfer(address indexed from, address indexed to, uint256 value)
    let transfer_topic = FixedBytes::<32>::from_str(
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
    )
    .unwrap();
    // WETH Withdrawal(address indexed src, uint256 wad)
    let withdrawal_topic = FixedBytes::<32>::from_str(
        "0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65",
    )
    .unwrap();
    // V4 Swap(bytes32 indexed id, address indexed sender, int128 amount0, int128 amount1, ...)
    let swap_topic = FixedBytes::<32>::from_str(
        "0x40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f",
    )
    .unwrap();

    let mut confirmed = 0u32;

    for ct in &confirmed_txs {
        let tx = &ct.entry.tx;
        let receipt = &ct.receipt;
        let block_number = receipt.block_number.unwrap_or(0) as i64;
        let is_trade = tx.tx_type == "BUY" || tx.tx_type == "SELL";

        let mut actual_value: Option<f64> = None;
        let mut actual_tokens: Option<f64> = None;

        // Parse actual amounts from receipt logs (same logic as exec_transaction in uniswap.rs)
        if receipt.status() && is_trade {
            let sender_addr = tx.address.parse::<Address>().ok();
            let token_out_addr = tx.token_out.parse::<Address>().ok();

            if tx.tx_type == "BUY" {
                // BUY: value (ETH spent) is already exact, parse actual tokens received
                if let (Some(sender), Some(token_out)) = (sender_addr, token_out_addr) {
                    for log in receipt.inner.logs() {
                        if log.inner.address == token_out
                            && log.inner.data.topics().len() >= 3
                            && log.inner.data.topics()[0] == transfer_topic
                        {
                            let to = Address::from_word(log.inner.data.topics()[2]);
                            if to == sender {
                                let raw = U256::from_be_slice(log.inner.data.data.as_ref());
                                let decimals =
                                    decimals_map.get(&tx.token_out).copied().unwrap_or(1e18);
                                actual_tokens = Some(u256_to_f64(raw) / decimals);
                            }
                        }
                    }
                }
            } else {
                // SELL: tokens (tokens sent) is already exact, parse actual ETH received
                if let Some(token_out) = token_out_addr {
                    // Try WETH Withdrawal first (V2 sells unwrap WETH → ETH)
                    for log in receipt.inner.logs() {
                        if log.inner.address == token_out
                            && !log.inner.data.topics().is_empty()
                            && log.inner.data.topics()[0] == withdrawal_topic
                        {
                            let raw = U256::from_be_slice(log.inner.data.data.as_ref());
                            actual_value = Some(u256_to_f64(raw) / 1e18);
                        }
                    }

                    // V4 fallback: native ETH output — parse Swap event
                    if actual_value.is_none() {
                        for log in receipt.inner.logs() {
                            if !log.inner.data.topics().is_empty()
                                && log.inner.data.topics()[0] == swap_topic
                                && log.inner.data.data.len() >= 32
                            {
                                let amount0_bytes: [u8; 32] =
                                    log.inner.data.data[0..32].try_into().unwrap();
                                if amount0_bytes[0] & 0x80 == 0 {
                                    let amount0 = U256::from_be_bytes(amount0_bytes);
                                    if amount0 > U256::ZERO {
                                        actual_value = Some(u256_to_f64(amount0) / 1e18);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Update DB with block number + actual values
        let value_bd = actual_value.map(f64_to_big_int);
        let tokens_bd = actual_tokens.map(f64_to_big_int);

        if let Err(e) = db
            .update_transaction_slot(&tx.tx_hash, block_number, value_bd, tokens_bd)
            .await
        {
            tracing::warn!(
                "[EVM] Failed to update slot for tx {}: {}",
                tx.tx_hash,
                e
            );
            errors += 1;
            continue;
        }

        // Update daily volume cache
        let date = chrono::DateTime::<chrono::Utc>::from(tx.date_added)
            .format("%Y-%m-%d")
            .to_string();
        let volume = actual_value
            .unwrap_or_else(|| tx.value.to_f64().unwrap_or(0.0));
        if volume > 0.0 {
            let _ =
                crate::cache::add_daily_volume_for_date(ct.entry.tx.project_id, &date, volume)
                    .await;
        }

        tracing::debug!(
            "[EVM] Confirmed tx {} (type={}, block={}, value={}, tokens={}, volume={:.4})",
            tx.tx_hash,
            tx.tx_type,
            block_number,
            actual_value.map(|v| format!("{:.6}", v)).unwrap_or_else(|| "unchanged".into()),
            actual_tokens.map(|t| format!("{:.6}", t)).unwrap_or_else(|| "unchanged".into()),
            volume
        );

        confirmed += 1;
    }

    tracing::info!(
        "[EVM] Resolved pending txs: {} confirmed, {} deleted, {} errors",
        confirmed,
        deleted,
        errors
    );
}
