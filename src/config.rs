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
        }
    }
}
