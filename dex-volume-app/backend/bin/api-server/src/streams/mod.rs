use std::sync::Arc;

use vm_data::models::{
    streams::{StreamInfo, StreamType},
    ws::WsBroadcastGeneric,
};

use crate::models::state::AppState;

/// Strip `:buy` / `:sell` suffixes from bundle tx hashes before sending to clients.
/// DB stores suffixed hashes for uniqueness; clients see clean hashes for block explorer links.
pub fn strip_tx_hash_suffix(tx_hash: &str) -> String {
    tx_hash
        .strip_suffix(":buy")
        .or_else(|| tx_hash.strip_suffix(":sell"))
        .unwrap_or(tx_hash)
        .to_string()
}

/// Handle DB writes for transaction events, then broadcast the result to WS clients.
///
/// Only one api-server instance executes this per event (via NATS queue group).
/// Handles `NewTransactionRequest`, `NewTransactionsRequest`, and `UpdateTransactionSlot`.
/// After a successful DB write, broadcasts the result to WS clients connected to this instance.
pub async fn process_event_db_write(state: Arc<AppState>, stream_info: StreamInfo) {
    let db = state.get_db();
    match stream_info.stream_type {
        StreamType::NewTransactionRequest(new_tx) => {
            let project_id = new_tx.project_id;
            let tx_hash = new_tx.tx_hash.clone();
            let t0 = std::time::Instant::now();

            // Confirmed tx (slot > 0, e.g. from Geyser): try update first for pre-inserted
            // bundle txs, then fall back to insert for non-bundle txs.
            if new_tx.slot > 0
                && let Ok(Some(mut tx)) = db
                    .update_transaction_slot(&tx_hash, new_tx.slot, Some(new_tx.value.clone()), Some(new_tx.tokens.clone()))
                    .await
                {
                    tracing::debug!(
                        "[DB] update_transaction (confirmed) | project={} | tx={} | slot={} | elapsed={}ms",
                        project_id, tx_hash, new_tx.slot, t0.elapsed().as_millis(),
                    );
                    tx.tx_hash = strip_tx_hash_suffix(&tx.tx_hash);
                    let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                        r#type: "UPDATE_TX".to_string(),
                        data: StreamType::UpdateTransactionResponse(tx),
                        id: Some(stream_info.user_id),
                        message: None,
                    };
                    state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
                    return;
                }

            // No pre-existing row (non-bundle tx) or slot=0 (pre-insert): insert new row
            match db.add_transaction(&new_tx).await {
                Ok(Some(mut tx)) => {
                    tracing::info!(
                        "[DB] add_transaction | project={} | tx={} | elapsed={}ms",
                        project_id, tx_hash, t0.elapsed().as_millis(),
                    );
                    tx.tx_hash = strip_tx_hash_suffix(&tx.tx_hash);
                    let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                        r#type: "NEW_TX".to_string(),
                        data: StreamType::NewTransactionResponse(tx),
                        id: Some(stream_info.user_id),
                        message: None,
                    };
                    state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
                }
                Ok(None) => {
                    tracing::warn!(
                        "[DB] add_transaction returned None (conflict) | project={} | tx={} | elapsed={}ms",
                        project_id, tx_hash, t0.elapsed().as_millis(),
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "[DB] add_transaction failed | project={} | tx={} | elapsed={}ms | error={}",
                        project_id, tx_hash, t0.elapsed().as_millis(), e,
                    );
                }
            }
        }
        StreamType::NewTransactionsRequest(txs) => {
            let project_id = txs.first().map(|t| t.project_id).unwrap_or(0);
            let t0 = std::time::Instant::now();
            match db.add_transactions(&txs).await {
                Ok(inserted) => {
                    tracing::info!(
                        "[DB] add_transactions | project={} | count={} | elapsed={}ms",
                        project_id, inserted.len(), t0.elapsed().as_millis(),
                    );
                    for mut tx in inserted {
                        tx.tx_hash = strip_tx_hash_suffix(&tx.tx_hash);
                        let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                            r#type: "NEW_TX".to_string(),
                            data: StreamType::NewTransactionResponse(tx),
                            id: Some(stream_info.user_id),
                            message: None,
                        };
                        state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
                    }
                }
                Err(e) => {
                    tracing::error!("[DB] add_transactions failed | project={} | error={}", project_id, e);
                }
            }
        }
        StreamType::UpdateTransactionSlot { tx_hash, slot, value, tokens } => {
            if let Some(mut tx) = db.update_transaction_slot(&tx_hash, slot, value, tokens).await.unwrap_or(None) {
                tx.tx_hash = strip_tx_hash_suffix(&tx.tx_hash);
                let project_id = tx.project_id;
                let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                    r#type: "UPDATE_TX".to_string(),
                    data: StreamType::UpdateTransactionResponse(tx),
                    id: Some(stream_info.user_id),
                    message: None,
                };
                state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
            }
        }
        _ => {}
    }
}

/// Broadcast WS-only events to connected clients on this instance.
///
/// Every api-server instance executes this for every event (plain NATS subscribe).
/// Handles non-DB stream types: BalanceUpdate, DailyVolumeUpdate, ResponsePrice,
/// TaskStatusUpdate, ResponseWalletsFinancials. DB-writing types are a no-op here
/// (handled by [`process_event_db_write`] via queue subscription).
pub async fn process_event_ws_broadcast(state: Arc<AppState>, stream_info: StreamInfo, price_type: Option<&str>) {
    match stream_info.stream_type {
        StreamType::BalanceUpdate(balance_update) => {
            let project_id = balance_update.relation.project_id;
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "UPDATE_BALANCE".to_string(),
                data: StreamType::BalanceUpdate(balance_update),
                id: Some(stream_info.user_id),
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
        }
        StreamType::DailyVolumeUpdate(info) => {
            let project_id = info.project_id;
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "DAILY_VOLUME".to_string(),
                data: StreamType::DailyVolumeUpdate(info),
                id: Some(stream_info.user_id),
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
        }
        StreamType::ResponsePrice(price) => {
            // Price events are global — broadcast to all clients
            let ws_type = price_type.unwrap_or("SOL_PRICE");
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: ws_type.to_string(),
                data: StreamType::ResponsePrice(price),
                id: None,
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, None).await;
        }
        StreamType::TaskStatusUpdate(ref info) => {
            let project_id = info.project_id;
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "TASK_STATUS".to_string(),
                data: StreamType::TaskStatusUpdate(info.clone()),
                id: Some(stream_info.user_id),
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
        }
        StreamType::ResponseWalletsFinancials(ref response) => {
            let project_id = response.project_id;
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "WALLETS_FINANCIALS".to_string(),
                data: StreamType::ResponseWalletsFinancials(response.clone()),
                id: None,
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, Some(project_id)).await;
        }
        // DB-writing types (NewTransactionRequest, NewTransactionsRequest,
        // UpdateTransactionSlot) are handled by process_event_db_write via
        // queue subscription — no-op here.
        _ => {}
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_suffix_buy() {
        assert_eq!(strip_tx_hash_suffix("0xabc123:buy"), "0xabc123");
    }

    #[test]
    fn strip_suffix_sell() {
        assert_eq!(strip_tx_hash_suffix("0xabc123:sell"), "0xabc123");
    }

    #[test]
    fn strip_suffix_none() {
        assert_eq!(strip_tx_hash_suffix("0xabc123"), "0xabc123");
    }

    #[test]
    fn strip_suffix_empty() {
        assert_eq!(strip_tx_hash_suffix(""), "");
    }

    #[test]
    fn strip_suffix_only_first_match() {
        // :buy:sell should only strip the outer :sell
        assert_eq!(strip_tx_hash_suffix("0xabc:buy:sell"), "0xabc:buy");
    }

    #[test]
    fn strip_suffix_preserves_interior() {
        // "buy" or "sell" not at the end should not be stripped
        assert_eq!(strip_tx_hash_suffix("buy_0xabc"), "buy_0xabc");
        assert_eq!(strip_tx_hash_suffix("0xsellout"), "0xsellout");
    }
}