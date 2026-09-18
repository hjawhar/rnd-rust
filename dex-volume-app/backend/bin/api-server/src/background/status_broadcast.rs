use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use vm_data::models::ws::WsBroadcastGeneric;
use crate::models::state::AppState;
use super::try_acquire_lock;

pub fn spawn_status_broadcast(state: Arc<AppState>, shutdown: CancellationToken) {
    tokio::task::spawn(async move {
        let mut prev_health_status: Option<&str> = None;
        let mut prev_sol_count: Option<usize> = None;
        let mut prev_evm_count: Option<usize> = None;

        loop {
            let status = crate::routes::health::compute_system_status(&state).await;

            // ── Systematic alerting: detect state transitions ────────
            let cur_health = status.health.status;
            let cur_sol = status.workers.sol.active;
            let cur_evm = status.workers.evm.active;

            let mut alerts: Vec<String> = Vec::new();

            // Health status transitions
            if let Some(prev) = prev_health_status
                && prev != cur_health {
                    let checks = &status.health.checks;
                    let detail = format!("db={} nats={} redis={}", checks.db, checks.nats, checks.redis);
                    alerts.push(format!(
                        "\u{26a0}\u{fe0f} **System health: {} \u{2192} {}** ({})",
                        prev, cur_health, detail
                    ));
                }

            // Worker count transitions
            if let Some(prev) = prev_sol_count {
                if prev > 0 && cur_sol == 0 {
                    alerts.push("\u{1f534} **All SOL workers offline**".to_string());
                } else if prev == 0 && cur_sol > 0 {
                    alerts.push(format!("\u{1f7e2} **SOL workers recovered** ({} active)", cur_sol));
                }
            }
            if let Some(prev) = prev_evm_count {
                if prev > 0 && cur_evm == 0 {
                    alerts.push("\u{1f534} **All EVM workers offline**".to_string());
                } else if prev == 0 && cur_evm > 0 {
                    alerts.push(format!("\u{1f7e2} **EVM workers recovered** ({} active)", cur_evm));
                }
            }

            // Publish alerts to Discord — only one instance should publish per cycle
            if !alerts.is_empty() && try_acquire_lock("api:status_alert", 13).await {
                for alert in &alerts {
                    tracing::warn!("[ALERT] {}", alert);
                    let _ = state
                        .nats_client
                        .publish(
                            vm_nats::subjects::system::ALERT,
                            alert.clone().into(),
                        )
                        .await;
                }
            }

            prev_health_status = Some(cur_health);
            prev_sol_count = Some(cur_sol);
            prev_evm_count = Some(cur_evm);

            // ── Broadcast to WebSocket clients ───────────────────────
            let ws_broadcast_payload = WsBroadcastGeneric {
                r#type: "SYSTEM_STATUS".to_string(),
                data: status,
                id: None,
                message: None,
            };
            state.broadcast(&ws_broadcast_payload, None).await;

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
                _ = shutdown.cancelled() => break,
            }
        }
    });
}
