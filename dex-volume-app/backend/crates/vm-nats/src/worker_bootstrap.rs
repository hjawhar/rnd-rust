use tokio_util::sync::CancellationToken;

/// Spawn a task that cancels `shutdown` on SIGTERM or Ctrl+C.
///
/// Both worker binaries (sol, evm) use identical shutdown logic.
/// This extracts that into a shared helper.
pub fn spawn_shutdown_handler() -> CancellationToken {
    let shutdown = CancellationToken::new();
    let s = shutdown.clone();
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => tracing::info!("Received Ctrl+C"),
            _ = sigterm.recv() => tracing::info!("Received SIGTERM"),
        }
        s.cancel();
    });
    shutdown
}
