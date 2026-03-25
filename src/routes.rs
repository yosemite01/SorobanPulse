use axum::{response::IntoResponse, routing::get, Json, Router};
use http::{HeaderValue, Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{handlers, middleware};

pub fn create_router(
    pool: PgPool,
    api_key: Option<String>,
    allowed_origins: &[String],
    rate_limit_per_minute: u32,
) -> Router {
    let cors = build_cors(allowed_origins);
    let auth_state = Arc::new(middleware::AuthState { api_key });

    // Replenish one token every (60 / rate_limit_per_minute) seconds.
    // burst_size = rate_limit_per_minute so a fresh client can use the full quota at once.
    let period_secs = 60u64.div_ceil(rate_limit_per_minute as u64);
    let governor_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(period_secs)
            .burst_size(rate_limit_per_minute)
            .use_headers()
            .error_handler(|_| {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({ "error": "rate limit exceeded" })),
                )
                    .into_response()
            })
            .finish()
            .expect("invalid rate limit configuration"),
    );

    // Rate-limited API routes
    let api = Router::new()
        .route("/events", get(handlers::get_events))
        .route("/events/:contract_id", get(handlers::get_events_by_contract))
        .route("/events/tx/:tx_hash", get(handlers::get_events_by_tx))
        .layer(GovernorLayer::new(governor_conf));

    Router::new()
        .route("/health", get(handlers::health))
        .merge(api)
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth_middleware,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(pool)
}

fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let methods = [Method::GET];

    if allowed_origins.iter().any(|o| o == "*") {
        return CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(methods);
    }

    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(methods)
        .vary([http::header::ORIGIN])
}
