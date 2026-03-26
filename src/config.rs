use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use url::Url;

/// Shared state for health checks, accessible between the indexer and HTTP handlers
#[derive(Clone)]
pub struct HealthState {
    /// Unix timestamp of the last successful indexer poll
    pub last_indexer_poll: Arc<AtomicU64>,
    /// Timeout in seconds after which the indexer is considered stalled
    pub indexer_stall_timeout_secs: u64,
}

impl HealthState {
    pub fn new(indexer_stall_timeout_secs: u64) -> Self {
        Self {
            last_indexer_poll: Arc::new(AtomicU64::new(0)),
            indexer_stall_timeout_secs,
        }
    }

    /// Update the last poll timestamp to the current time
    pub fn update_last_poll(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_indexer_poll.store(now, Ordering::SeqCst);
    }

    /// Check if the indexer is stalled (no successful poll within the timeout)
    /// Returns Some(seconds_ago) if stalled, None if OK
    pub fn is_indexer_stalled(&self) -> Option<u64> {
        let last_poll = self.last_indexer_poll.load(Ordering::SeqCst);
        if last_poll == 0 {
            // No poll ever completed
            return Some(0);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let elapsed = now.saturating_sub(last_poll);
        if elapsed > self.indexer_stall_timeout_secs {
            Some(elapsed)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub stellar_rpc_url: String,
    pub start_ledger: u64,
    pub start_ledger_fallback: bool,
    pub port: u16,
    pub api_key: Option<String>,
    pub db_max_connections: u32,
    pub db_min_connections: u32,
    pub behind_proxy: bool,
    pub rpc_connect_timeout_secs: u64,
    pub rpc_request_timeout_secs: u64,
    pub allowed_origins: Vec<String>,
    pub rate_limit_per_minute: u32,
    pub indexer_lag_warn_threshold: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "postgres://localhost/unused".to_string(),
            stellar_rpc_url: "http://localhost".to_string(),
            start_ledger: 0,
            start_ledger_fallback: false,
            port: 3000,
            api_key: None,
            db_max_connections: 10,
            db_min_connections: 1,
            behind_proxy: false,
            rpc_connect_timeout_secs: 5,
            rpc_request_timeout_secs: 30,
            allowed_origins: vec!["*".to_string()],
            rate_limit_per_minute: 60,
            indexer_stall_timeout_secs: 60,
            indexer_lag_warn_threshold: 100,
        }
    }
}

fn validate_rpc_url(raw: &str) -> String {
    let url = Url::parse(raw)
        .unwrap_or_else(|e| panic!("STELLAR_RPC_URL is not a valid URL: {e}"));

    let allow_insecure = env::var("ALLOW_INSECURE_RPC")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "y"))
        .unwrap_or(false);

    match url.scheme() {
        "https" => {}
        "http" if allow_insecure => {}
        "http" => panic!(
            "STELLAR_RPC_URL uses http — set ALLOW_INSECURE_RPC=true to permit insecure connections"
        ),
        scheme => panic!("STELLAR_RPC_URL has disallowed scheme '{scheme}' — only https is permitted"),
    }

    if !allow_insecure {
        let host = url.host_str().unwrap_or("");
        let is_loopback = host == "localhost"
            || host == "127.0.0.1"
            || host == "::1"
            || host.ends_with(".local");
        let is_private = {
            // Reject RFC-1918 / link-local ranges by prefix
            host.starts_with("10.")
                || host.starts_with("192.168.")
                || host.starts_with("169.254.")
                || (host.starts_with("172.") && {
                    host.split('.')
                        .nth(1)
                        .and_then(|o| o.parse::<u8>().ok())
                        .map(|o| (16..=31).contains(&o))
                        .unwrap_or(false)
                })
        };
        if is_loopback || is_private {
            panic!(
                "STELLAR_RPC_URL points to a non-routable host '{host}' — \
                 set ALLOW_INSECURE_RPC=true to allow this in development"
            );
        }
    }

    // Return URL without credentials
    let mut safe = url.clone();
    let _ = safe.set_username("");
    let _ = safe.set_password(None);
    safe.to_string()
}

impl Config {
    pub fn from_env() -> Self {
        let behind_proxy = env::var("BEHIND_PROXY")
            .ok()
            .map(|v| {
                let v = v.to_ascii_lowercase();
                matches!(v.as_str(), "true" | "1" | "yes" | "y")
            })
            .unwrap_or(false);

        let start_ledger = env::var("START_LEDGER")
            .unwrap_or_else(|_| "0".to_string())
            .parse()
            .expect("START_LEDGER must be a number");

        let start_ledger_fallback = env::var("START_LEDGER_FALLBACK")
            .ok()
            .map(|v| {
                let v = v.to_ascii_lowercase();
                matches!(v.as_str(), "true" | "1" | "yes" | "y")
            })
            .unwrap_or(false);

        let port = env::var("PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .expect("PORT must be a number");

        Self {
            database_url: env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
            stellar_rpc_url: validate_rpc_url(
                &env::var("STELLAR_RPC_URL")
                    .unwrap_or_else(|_| "https://soroban-testnet.stellar.org".to_string()),
            ),
            start_ledger,
            start_ledger_fallback,
            port,
            api_key: env::var("API_KEY").ok(),
            db_max_connections: env::var("DB_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .expect("DB_MAX_CONNECTIONS must be a number"),
            db_min_connections: env::var("DB_MIN_CONNECTIONS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()
                .expect("DB_MIN_CONNECTIONS must be a number"),
            behind_proxy,
            rpc_connect_timeout_secs: env::var("RPC_CONNECT_TIMEOUT_SECS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()
                .expect("RPC_CONNECT_TIMEOUT_SECS must be a number"),
            rpc_request_timeout_secs: env::var("RPC_REQUEST_TIMEOUT_SECS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .expect("RPC_REQUEST_TIMEOUT_SECS must be a number"),
            allowed_origins: env::var("ALLOWED_ORIGINS")
                .unwrap_or_else(|_| "*".to_string())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            rate_limit_per_minute: env::var("RATE_LIMIT_PER_MINUTE")
                .unwrap_or_else(|_| "60".to_string())
                .parse()
                .expect("RATE_LIMIT_PER_MINUTE must be a positive integer"),
            indexer_lag_warn_threshold: env::var("INDEXER_LAG_WARN_THRESHOLD")
                .unwrap_or_else(|_| "100".to_string())
                .parse()
                .expect("INDEXER_LAG_WARN_THRESHOLD must be a number"),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_url: "postgres://localhost/soroban_pulse".to_string(),
            stellar_rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            start_ledger: 0,
            start_ledger_fallback: false,
            port: 8080,
            api_key: None,
            db_max_connections: 10,
            db_min_connections: 1,
            behind_proxy: false,
            rpc_connect_timeout_secs: 5,
            rpc_request_timeout_secs: 30,
            allowed_origins: vec!["*".to_string()],
            rate_limit_per_minute: 60,
            indexer_lag_warn_threshold: 100,
        }
    }
}
