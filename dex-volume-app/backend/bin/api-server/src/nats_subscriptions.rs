use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vm_data::models::streams::StreamInfo;
use vm_nats::subjects;
use vm_nats::subscribe_loop::{spawn_subscribe_loop, spawn_broadcast_loop};
use crate::models::state::AppState;
use crate::streams::{process_event_db_write, process_event_ws_broadcast};

pub fn setup(
    nats_client: &async_nats::Client,
    state: Arc<AppState>,
    shutdown: CancellationToken,
) {
    // ── Queue-subscribed DB writers for transaction subjects ────────────────
    // Only one instance per queue group processes each message, eliminating
    // redundant DB writes across N api-server instances.
    for subject in [
        subjects::events::sol::TRANSACTION_NEW,
        subjects::events::evm::TRANSACTION_NEW,
    ] {
        let state = state.clone();
        spawn_subscribe_loop(
            nats_client,
            subject,
            "event-db-writer",
            shutdown.clone(),
            Some(50),
            move |_client, msg| {
                let state = state.clone();
                async move {
                    if let Ok(stream_info) = serde_json::from_slice::<StreamInfo>(&msg.payload) {
                        process_event_db_write(state, stream_info).await;
                    }
                }
            },
        );
    }

    // ── Broadcast subscriptions for WS fan-out ─────────────────────────────
    // Every instance receives every message and broadcasts to its own WS clients.
    // Non-price subjects don't need price_type context.
    for subject in [
        subjects::events::sol::BALANCE_UPDATE,
        subjects::events::sol::TRANSACTION_NEW,
        subjects::events::sol::DAILY_VOLUME,
        subjects::events::sol::TASK_STATUS,
        subjects::events::sol::WALLETS_FINANCIALS,
        subjects::events::evm::BALANCE_UPDATE,
        subjects::events::evm::TRANSACTION_NEW,
        subjects::events::evm::DAILY_VOLUME,
        subjects::events::evm::TASK_STATUS,
        subjects::events::evm::WALLETS_FINANCIALS,
    ] {
        let state = state.clone();
        spawn_broadcast_loop(
            nats_client,
            subject,
            shutdown.clone(),
            None,
            move |_client, msg| {
                let state = state.clone();
                async move {
                    if let Ok(stream_info) = serde_json::from_slice::<StreamInfo>(&msg.payload) {
                        process_event_ws_broadcast(state, stream_info, None).await;
                    }
                }
            },
        );
    }

    // Price event subjects need price_type context to distinguish SOL vs ETH
    for (subject, price_type) in [
        (subjects::events::sol::PRICE, "SOL_PRICE"),
        (subjects::events::evm::PRICE, "ETH_PRICE"),
    ] {
        let state = state.clone();
        let price_type = price_type.to_string();
        spawn_broadcast_loop(
            nats_client,
            subject,
            shutdown.clone(),
            None,
            move |_client, msg| {
                let state = state.clone();
                let pt = price_type.clone();
                async move {
                    if let Ok(stream_info) = serde_json::from_slice::<StreamInfo>(&msg.payload) {
                        process_event_ws_broadcast(state, stream_info, Some(&pt)).await;
                    }
                }
            },
        );
    }
}
