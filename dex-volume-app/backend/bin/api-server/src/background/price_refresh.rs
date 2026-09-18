use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use vm_data::models::streams::StreamType;
use vm_data::models::ws::WsBroadcastGeneric;
use crate::models::state::AppState;
use super::try_acquire_lock;

pub fn spawn_price_refresh(
    state: Arc<AppState>,
    shutdown: CancellationToken,
    subject: &'static str,
    lock_key: &'static str,
    ws_type: &'static str,
) {
    tokio::task::spawn(async move {
        loop {
            // Acquire distributed lock so only one instance refreshes the price.
            // TTL 280s < 300s interval — lock expires before next tick.
            if try_acquire_lock(lock_key, 280).await
                && let Ok(msg) = vm_nats::request_with_timeout(
                    &state.nats_client,
                    subject,
                    vec![],
                    Duration::from_secs(5),
                )
                .await
                    && let Ok(stream_type) = serde_json::from_slice::<StreamType>(&msg.payload)
                    && let StreamType::ResponsePrice(price) = stream_type
                {
                    let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                        r#type: ws_type.to_string(),
                        data: StreamType::ResponsePrice(price),
                        id: None,
                        message: None,
                    };
                    state.broadcast(&ws_broadcast_payload, None).await;
                }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(300)) => {}
                _ = shutdown.cancelled() => break,
            }
        }
    });
}
