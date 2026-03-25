mod config;
mod db;
mod error;
mod handlers;
mod indexer;
mod middleware;
mod models;
mod routes;

use std::net::SocketAddr;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::from_env();
    
    let pool = {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match db::create_pool(
                &config.database_url,
                config.db_max_connections,
                config.db_min_connections,
            )
            .await
            {
                Ok(p) => break p,
                Err(e) => {
                    if attempt >= 3 {
                        tracing::error!("Failed to connect to database after 3 attempts: {}", e);
                        std::process::exit(1);
                    }
                    tracing::warn!(attempt = attempt, "DB connection failed, retrying...");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    };
    
    let _ = db::run_migrations(&pool).await;

    info!("Migrations applied successfully");
    info!("Soroban RPC URL: {}", config.stellar_rpc_url);

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut shutdown_rx_axum = shutdown_rx.clone();

    // Spawn background indexer
    let indexer = indexer::Indexer::new(pool.clone(), config.clone(), shutdown_rx);
    let indexer_handle = tokio::spawn(async move {
        indexer.run().await;
    });

    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
        tracing::info!("Shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!("Allowed CORS origins: {:?}", config.allowed_origins);
    info!("Rate limit: {} requests/minute per IP", config.rate_limit_per_minute);
    let router = routes::create_router(pool, config.api_key, &config.allowed_origins, config.rate_limit_per_minute);

    info!("Soroban Pulse listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        error!("Address already in use");
        e
    })?;

    info!("Running server - trusting X-Forwarded-For: {}", config.behind_proxy);

    // GovernorLayer requires connect_info to extract peer IP — always use it.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx_axum.changed().await;
    })
    .await?;
    let _ = indexer_handle.await;

    Ok(())
}
