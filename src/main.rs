mod config;
mod db;
mod error;
mod handlers;
mod indexer;
mod metrics;
mod middleware;
mod models;
mod routes;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[cfg(feature = "otel")]
use opentelemetry::global;
#[cfg(feature = "otel")]
use opentelemetry_otlp::WithExportConfig;
#[cfg(feature = "otel")]
use tracing_opentelemetry::OpenTelemetryLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let log_format = std::env::var("RUST_LOG_FORMAT").unwrap_or_else(|_| "text".to_string());
    
    let registry = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()));

    #[cfg(feature = "otel")]
    let registry = {
        let otel_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:4317".to_string());
        
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(otel_endpoint),
            )
            .install_simple()
            .expect("Failed to initialize OpenTelemetry tracer");
        
        registry.with(OpenTelemetryLayer::new(tracer))
    };

    if log_format == "json" {
        registry
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        registry
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    // Initialize metrics exporter
    let prometheus_handle = metrics::init_metrics();

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
                        tracing::error!(error = %e, "Failed to connect to database after 3 attempts");
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
    info!(url = %config.stellar_rpc_url, "Soroban RPC URL");

    // Create shared health state for indexer and HTTP handlers
    let health_state = Arc::new(config::HealthState::new(config.indexer_stall_timeout_secs));

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut shutdown_rx_axum = shutdown_rx.clone();

    // Spawn background indexer with health state
    let mut indexer = indexer::Indexer::new(pool.clone(), config.clone(), shutdown_rx);
    indexer.set_health_state(health_state.clone());
    let indexer_handle = tokio::spawn(async move {
        indexer.run().await;
    });

    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.ok();
        }
        tracing::info!("Shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!(origins = ?config.allowed_origins, "Allowed CORS origins");
    info!(rate_limit = config.rate_limit_per_minute, "Rate limit per IP");
    let router = routes::create_router(pool, config.api_key, &config.allowed_origins, config.rate_limit_per_minute, prometheus_handle);

    info!(addr = %addr, "Soroban Pulse listening");

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        error!(addr = %addr, "Address already in use");
        e
    })?;

    info!(behind_proxy = config.behind_proxy, "Running server - trusting X-Forwarded-For");

    // Use regular make_service since we handle connect_info through middleware
    // Use the router directly as it implements Service for incoming connections
    axum::serve(
        listener,
        router,
    )
    .with_graceful_shutdown(async move {
        let _ = shutdown_rx_axum.changed().await;
    })
    .await?;
    let _ = indexer_handle.await;

    #[cfg(feature = "otel")]
    global::shutdown_tracer_provider();

    Ok(())
}
