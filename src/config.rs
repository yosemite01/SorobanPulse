use anyhow::{Context, Result};
use std::env;

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
            stellar_rpc_url: env::var("STELLAR_RPC_URL")
                .unwrap_or_else(|_| "https://soroban-testnet.stellar.org".to_string()),
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
        }
    }
}
