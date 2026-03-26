use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Initialize the Prometheus metrics exporter
pub fn init_metrics() -> PrometheusHandle {
    let builder = PrometheusBuilder::new();
    builder
        .install()
        .expect("Failed to install Prometheus exporter")
}

/// Record events indexed
pub fn record_events_indexed(count: u64) {
    metrics::counter!("soroban_pulse_events_indexed_total").increment(count);
}

/// Update the current ledger being processed
pub fn update_current_ledger(ledger: u64) {
    metrics::gauge!("soroban_pulse_indexer_current_ledger").set(ledger as f64);
}

/// Update the latest ledger from RPC
pub fn update_latest_ledger(ledger: u64) {
    metrics::gauge!("soroban_pulse_indexer_latest_ledger").set(ledger as f64);
}

/// Update the indexer lag
pub fn update_indexer_lag(lag: u64) {
    metrics::gauge!("soroban_pulse_indexer_lag_ledgers").set(lag as f64);
}

/// Record an RPC error
pub fn record_rpc_error() {
    metrics::counter!("soroban_pulse_rpc_errors_total").increment(1);
}

/// Record HTTP request duration
pub fn record_http_request_duration(duration: std::time::Duration, method: &str, route: &str, status: &str) {
    metrics::histogram!("soroban_pulse_http_request_duration_seconds", "method" => method.to_string(), "route" => route.to_string(), "status" => status.to_string())
        .record(duration.as_secs_f64());
}
