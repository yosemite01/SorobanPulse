use anyhow::{Context, Result};
use std::env;
use url::Url;

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
    pub allowed_origins: Vec<String>,
    pub rate_limit_per_minute: u32,
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
        }
    }
}
