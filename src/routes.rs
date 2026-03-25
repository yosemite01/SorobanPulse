use axum::{routing::get, Router};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{handlers, middleware};

pub fn create_router(pool: PgPool, api_key: Option<String>) -> Router {
    let auth_state = Arc::new(middleware::AuthState { api_key });

    Router::new()
        .route("/health", get(handlers::health))
        .route("/events", get(handlers::get_events))
        .route("/events/:contract_id", get(handlers::get_events_by_contract))
        .route("/events/tx/:tx_hash", get(handlers::get_events_by_tx))
        .layer(axum::middleware::from_fn_with_state(auth_state, middleware::auth_middleware))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(pool)
}
