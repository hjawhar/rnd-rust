use vm_data::models::streams::{StreamInfo, StreamType};
use serde::Serialize;
use std::future::Future;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

/// Generic RPC handler that follows the same pattern as worker-sol.
/// Subscribes to a NATS subject with queue group, extracts payload from StreamInfo,
/// processes it, and replies with the result.
pub fn rpc_handler<P, R, F, Fut, Extract, Wrap>(
    nats_client: &async_nats::Client,
    queue_group: &str,
    subject: &str,
    extract: Extract,
    wrap: Wrap,
    handler: F,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()>
where
    P: Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(async_nats::Client, i32, P) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Option<R>> + Send + 'static,
    Extract: Fn(StreamType) -> Option<P> + Send + Sync + Clone + 'static,
    Wrap: Fn(Option<R>) -> StreamType + Send + Sync + Clone + 'static,
{
    let nc = nats_client.clone();
    let qg = queue_group.to_string();
    let subj = subject.to_string();

    tokio::task::spawn(async move {
        use futures::StreamExt;
        let mut sub = nc
            .queue_subscribe(subj.clone(), qg.clone())
            .await
            .unwrap_or_else(|e| panic!("Failed to subscribe to {}: {}", subj, e));

        loop {
            tokio::select! {
                msg = sub.next() => {
                    let Some(msg) = msg else {
                        tracing::warn!("[NATS] Subscription ended for {subj}, resubscribing in 1s...");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        match nc.queue_subscribe(subj.clone(), qg.clone()).await {
                            Ok(new_sub) => { sub = new_sub; continue; }
                            Err(e) => {
                                tracing::error!("[NATS] Resubscribe failed for {subj}: {e}");
                                break;
                            }
                        }
                    };
                    let Some(reply) = msg.reply.clone() else { continue };
                    let trace_id = vm_nats::extract_trace_id(&msg);
                    let Ok(stream_info) = serde_json::from_slice::<StreamInfo>(&msg.payload) else { continue };
                    let user_id = stream_info.user_id;

                    let Some(payload) = extract(stream_info.stream_type) else { continue };

                    let nc_inner = nc.clone();
                    let wrap_inner = wrap.clone();
                    let handler_inner = handler.clone();
                    let span = tracing::info_span!("rpc", subject = %subj, trace_id = trace_id.as_deref().unwrap_or("-"));

                    tokio::spawn(async move {
                        let result = handler_inner(nc_inner.clone(), user_id, payload).await;
                        let response = wrap_inner(result);
                        let bytes = serde_json::to_vec(&response).unwrap_or_default();
                        let _ = nc_inner.publish(reply, bytes.into()).await;
                    }.instrument(span));
                }
                _ = shutdown.cancelled() => break,
            }
        }
    })
}
