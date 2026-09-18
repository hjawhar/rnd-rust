//! Generic NATS subscription loop with auto-reconnect and graceful shutdown.
//!
//! Replaces the ~30-line boilerplate pattern repeated 15+ times across workers:
//! queue_subscribe → select! loop → handle None with resubscribe → break on shutdown.

use std::future::Future;
use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Spawn a NATS queue subscription loop.
///
/// Subscribes to `subject` in queue group `queue_group`, then loops:
/// - On message: calls `handler(client, message)` in a spawned task
/// - On subscription end (None): waits 1s, resubscribes
/// - On shutdown signal: breaks
///
/// The handler receives the raw `async_nats::Message` and full `async_nats::Client`.
/// Use this for custom dispatch logic. For typed RPC (StreamInfo extract/wrap),
/// prefer `rpc_handler` which builds on top of this.
pub fn spawn_subscribe_loop<F, Fut>(
    client: &async_nats::Client,
    subject: &'static str,
    queue_group: &str,
    shutdown: CancellationToken,
    max_concurrent: Option<usize>,
    handler: F,
) -> JoinHandle<()>
where
    F: Fn(async_nats::Client, async_nats::Message) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let client = client.clone();
    let queue_group = queue_group.to_string();
    let semaphore = max_concurrent.map(|n| Arc::new(Semaphore::new(n)));

    tokio::spawn(async move {
        let mut sub = client
            .queue_subscribe(subject, queue_group.clone())
            .await
            .unwrap_or_else(|e| panic!("Failed to subscribe to {subject}: {e}"));

        loop {
            tokio::select! {
                msg = sub.next() => {
                    let Some(msg) = msg else {
                        tracing::warn!("[NATS] Subscription ended for {subject}, resubscribing in 1s...");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        match client.queue_subscribe(subject, queue_group.clone()).await {
                            Ok(new_sub) => { sub = new_sub; continue; }
                            Err(e) => {
                                tracing::error!("[NATS] Resubscribe failed for {subject}: {e}");
                                break;
                            }
                        }
                    };
                    let client = client.clone();
                    let handler = handler.clone();
                    let sem = semaphore.clone();
                    tokio::spawn(async move {
                        let _permit = match sem {
                            Some(ref s) => Some(s.acquire().await.expect("semaphore closed")),
                            None => None,
                        };
                        handler(client, msg).await;
                    });
                }
                _ = shutdown.cancelled() => break,
            }
        }
    })
}

/// Spawn a plain (non-queue) NATS subscription loop.
///
/// Same as [`spawn_subscribe_loop`] but uses `subscribe()` instead of
/// `queue_subscribe()`. Every instance receives every message. Use for
/// event fan-out (e.g., WS broadcast).
pub fn spawn_broadcast_loop<F, Fut>(
    client: &async_nats::Client,
    subject: &'static str,
    shutdown: CancellationToken,
    max_concurrent: Option<usize>,
    handler: F,
) -> JoinHandle<()>
where
    F: Fn(async_nats::Client, async_nats::Message) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let client = client.clone();
    let semaphore = max_concurrent.map(|n| Arc::new(Semaphore::new(n)));

    tokio::spawn(async move {
        let mut sub = client
            .subscribe(subject)
            .await
            .unwrap_or_else(|e| panic!("Failed to subscribe to {subject}: {e}"));

        loop {
            tokio::select! {
                msg = sub.next() => {
                    let Some(msg) = msg else {
                        tracing::warn!("[NATS] Subscription ended for {subject}, resubscribing in 1s...");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        match client.subscribe(subject).await {
                            Ok(new_sub) => { sub = new_sub; continue; }
                            Err(e) => {
                                tracing::error!("[NATS] Resubscribe failed for {subject}: {e}");
                                break;
                            }
                        }
                    };
                    let client = client.clone();
                    let handler = handler.clone();
                    let sem = semaphore.clone();
                    tokio::spawn(async move {
                        let _permit = match sem {
                            Some(ref s) => Some(s.acquire().await.expect("semaphore closed")),
                            None => None,
                        };
                        handler(client, msg).await;
                    });
                }
                _ = shutdown.cancelled() => break,
            }
        }
    })
}
