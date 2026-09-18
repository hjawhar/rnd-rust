//! Prometheus metrics instrumentation.
//!
//! When enabled via [`MetricsConfig`](crate::config::MetricsConfig), the
//! global metrics recorder is installed at startup and a Prometheus HTTP
//! exporter is bound to the configured address. All metric helpers in
//! this module are no-ops when no recorder is installed.

use std::net::SocketAddr;

use metrics::{counter, gauge};
use metrics_exporter_prometheus::PrometheusBuilder;

/// Install the global Prometheus metrics recorder and HTTP exporter.
///
/// The exporter serves the standard `/metrics` endpoint on `bind`.
/// This function **must** be called at most once per process.
///
/// # Errors
///
/// Returns an error if the recorder cannot be installed (e.g. a
/// recorder is already registered).
pub fn install(bind: SocketAddr) -> Result<(), metrics_exporter_prometheus::BuildError> {
    PrometheusBuilder::new().with_http_listener(bind).install()
}

/// Increment the total-connections counter and the open-connections gauge.
pub fn record_connection_open() {
    counter!("irc_server_connections_total").increment(1);
    gauge!("irc_server_connections_open").increment(1.0);
}

/// Decrement the open-connections gauge.
pub fn record_connection_close() {
    gauge!("irc_server_connections_open").decrement(1.0);
}

/// Increment the per-command message counter.
pub fn record_message(command: &str) {
    counter!("irc_server_messages_total", "command" => command.to_owned()).increment(1);
}

/// Record an authentication attempt.
pub fn record_auth(mechanism: &str, success: bool) {
    let outcome = if success { "success" } else { "failure" };
    counter!(
        "irc_server_auth_total",
        "mechanism" => mechanism.to_owned(),
        "outcome" => outcome,
    )
    .increment(1);
}

/// Increment the flood-kick counter.
pub fn record_flood_kick() {
    counter!("irc_server_flood_kicks_total").increment(1);
}

/// Set the active k-lines gauge to the given value.
pub fn set_klines_active(count: f64) {
    gauge!("irc_server_klines_active").set(count);
}
