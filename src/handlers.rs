use axum::{extract::{Path, Query, State}, Json, response::IntoResponse};
use serde_json::{json, Value};
use sqlx::Row;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use metrics_exporter_prometheus::PrometheusHandle;

use crate::{config::HealthState, error::AppError, models::PaginationParams};

/// State type for the application that includes both DB pool and health state
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub health_state: Arc<HealthState>,
}

/// State type for health check endpoint - same as AppState
pub type HealthCheckState = AppState;

fn validate_contract_id(contract_id: &str) -> Result<(), AppError> {
    if contract_id.len() != 56 {
        return Err(AppError::Validation("invalid contract_id format".to_string()));
    }
    if !contract_id.starts_with('C') {
        return Err(AppError::Validation("invalid contract_id format".to_string()));
    }
    if !contract_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Validation("invalid contract_id format".to_string()));
    }
    Ok(())
}

fn validate_tx_hash(tx_hash: &str) -> Result<(), AppError> {
    if tx_hash.len() != 64 {
        return Err(AppError::Validation("invalid tx_hash format".to_string()));
    }
    if !tx_hash.chars().all(|c| c.is_ascii_hexdigit() && c.is_lowercase()) {
        return Err(AppError::Validation("invalid tx_hash format".to_string()));
    }
    Ok(())
}

/// Health check endpoint that verifies DB connectivity and indexer status
pub async fn health(State(state): State<HealthCheckState>) -> (axum::http::StatusCode, Json<Value>) {
    let mut db_ok = true;
    let mut db_reachable = true;

    // Check DB connectivity with 2-second timeout
    let db_check = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("SELECT 1")
            .fetch_one(&state.pool)
    );

    match db_check.await {
        Ok(Ok(_)) => {
            // DB is reachable
        }
        Ok(Err(_)) => {
            db_ok = false;
            db_reachable = false;
        }
        Err(_) => {
            // Timeout
            db_ok = false;
            db_reachable = false;
        }
    }

    // Check indexer status
    let indexer_status = if let Some(secs_ago) = state.health_state.is_indexer_stalled() {
        json!({
            "indexer": "stalled",
            "last_poll_secs_ago": secs_ago
        })
    } else {
        json!({"indexer": "ok"})
    };

    // Determine overall status
    let is_degraded = !db_ok || indexer_status.get("indexer").and_then(|v| v.as_str()) == Some("stalled");

    if is_degraded {
        let response = json!({
            "status": "degraded",
            "db": if db_reachable { "ok" } else { "unreachable" }
        });
        // Merge indexer status
        let mut obj = serde_json::to_value(response).unwrap();
        if let Value::Object(ref mut map) = obj {
            if let Value::Object(indexer_map) = indexer_status {
                map.extend(indexer_map);
            }
        }
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(obj))
    } else {
        let response = json!({
            "status": "ok",
            "db": "ok",
            "indexer": "ok"
        });
        (axum::http::StatusCode::OK, Json(response))
    }
}

pub async fn metrics(State(state): State<crate::routes::AppState>) -> impl IntoResponse {
    state.prometheus_handle.render()
}

pub async fn get_events(
    State(state): State<crate::routes::AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, AppError> {
    let pool = &state.pool;
    let limit = params.limit();
    let offset = params.offset();
    let exact = params.exact_count.unwrap_or(false);

    let columns = params.columns();

    let query_str = format!(
        "SELECT {} FROM events ORDER BY ledger DESC LIMIT $1 OFFSET $2",
        columns.join(", ")
    );

    let rows = sqlx::query(&query_str)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;

    let mut events = Vec::new();
    for row in rows {
        let mut event = serde_json::Map::new();
        for &col in &columns {
            match col {
                "id" => { event.insert(col.to_string(), json!(row.try_get::<Uuid, _>(col)?)); }
                "contract_id" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "event_type" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "tx_hash" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "ledger" => { event.insert(col.to_string(), json!(row.try_get::<i64, _>(col)?)); }
                "timestamp" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                "event_data" => { event.insert(col.to_string(), row.try_get::<Value, _>(col)?); }
                "created_at" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                _ => {}
            }
        }
        events.push(Value::Object(event));
    }

    let (total, approximate): (i64, bool) = if exact {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
            .fetch_one(&state.pool)
            .await?;
        (count, false)
    } else {
        let count: i64 = sqlx::query_scalar(
            "SELECT reltuples::bigint FROM pg_class WHERE relname = 'events'",
        )
        .fetch_one(&state.pool)
        .await?;
        (count, true)
    };

    Ok(Json(json!({
        "data": events,
        "total": total,
        "page": params.page.unwrap_or(1),
        "limit": limit,
        "approximate": approximate
    })))
}

pub async fn get_events_by_contract(
    State(state): State<crate::routes::AppState>,
    Path(contract_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, AppError> {
    let pool = &state.pool;
    validate_contract_id(&contract_id)?;
    
    let limit = params.limit();
    let offset = params.offset();
    let columns = params.columns();

    let query_str = format!(
        "SELECT {} FROM events WHERE contract_id = $1 ORDER BY ledger DESC LIMIT $2 OFFSET $3",
        columns.join(", ")
    );

    let rows = sqlx::query(&query_str)
        .bind(&contract_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;

    if rows.is_empty() {
        return Err(AppError::NotFound);
    }

    let mut events = Vec::new();
    for row in rows {
        let mut event = serde_json::Map::new();
        for &col in &columns {
            match col {
                "id" => { event.insert(col.to_string(), json!(row.try_get::<Uuid, _>(col)?)); }
                "contract_id" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "event_type" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "tx_hash" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "ledger" => { event.insert(col.to_string(), json!(row.try_get::<i64, _>(col)?)); }
                "timestamp" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                "event_data" => { event.insert(col.to_string(), row.try_get::<Value, _>(col)?); }
                "created_at" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                _ => {}
            }
        }
        events.push(Value::Object(event));
    }

    Ok(Json(json!({ "data": events, "contract_id": contract_id })))
}

pub async fn get_events_by_tx(
    State(state): State<crate::routes::AppState>,
    Path(tx_hash): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, AppError> {
    let pool = &state.pool;
    validate_tx_hash(&tx_hash)?;
    
    let columns = params.columns();
    let query_str = format!(
        "SELECT {} FROM events WHERE tx_hash = $1 ORDER BY ledger DESC",
        columns.join(", ")
    );

    let rows = sqlx::query(&query_str)
        .bind(&tx_hash)
        .fetch_all(&state.pool)
        .await?;

    let mut events = Vec::new();
    for row in rows {
        let mut event = serde_json::Map::new();
        for &col in &columns {
            match col {
                "id" => { event.insert(col.to_string(), json!(row.try_get::<Uuid, _>(col)?)); }
                "contract_id" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "event_type" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "tx_hash" => { event.insert(col.to_string(), json!(row.try_get::<String, _>(col)?)); }
                "ledger" => { event.insert(col.to_string(), json!(row.try_get::<i64, _>(col)?)); }
                "timestamp" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                "event_data" => { event.insert(col.to_string(), row.try_get::<Value, _>(col)?); }
                "created_at" => { event.insert(col.to_string(), json!(row.try_get::<DateTime<Utc>, _>(col)?)); }
                _ => {}
            }
        }
        events.push(Value::Object(event));
    }

    Ok(Json(json!({ "data": events, "tx_hash": tx_hash })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use sqlx::PgPool;
    use std::sync::Arc;
    use tower::ServiceExt;
    use crate::config::HealthState;

    fn create_test_router(pool: PgPool) -> impl axum::extract::InferRouteService<AppState> {
        let health_state = Arc::new(HealthState::new(60));
        crate::routes::create_router(pool, None, &[], 60, health_state)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_by_tx_no_events_returns_200_empty_data(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events/tx/unknown_tx_hash_no_events_deadbeef")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"], json!([]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_by_contract_no_events_returns_200_empty_data(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events/contract/unknown_contract_no_events_deadbeef")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"], json!([]));
        assert_eq!(v["contract_id"], json!("unknown_contract_no_events_deadbeef"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_by_tx_with_row_returns_200_with_data(pool: PgPool) {
        let tx_hash = "a1b2c3d4e5f6";
        sqlx::query(
            r#"
            INSERT INTO events (contract_id, event_type, tx_hash, ledger, timestamp, event_data)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind("C_TEST")
        .bind("contract")
        .bind(tx_hash)
        .bind(1_i64)
        .bind(Utc::now())
        .bind(json!({ "value": null, "topic": null }))
        .execute(&pool)
        .await
        .unwrap();

        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/events/tx/{tx_hash}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v["data"].is_array());
        assert_eq!(v["data"].as_array().unwrap().len(), 1);
        assert_eq!(v["tx_hash"], json!(tx_hash));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn database_error_response_does_not_leak_internals(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events?limit=invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        
        // Verify response contains generic error message
        assert!(body_str.contains("internal server error"));
        
        // Verify no SQLx internals are leaked
        assert!(!body_str.to_lowercase().contains("sqlx"));
        assert!(!body_str.contains("events"));
        assert!(!body_str.contains("table"));
        assert!(!body_str.contains("column"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn contract_id_too_long_returns_400(pool: PgPool) {
        let app = create_test_router(pool);
        let long_id = "C".repeat(100);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/events/{}", long_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "invalid contract_id format");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn contract_id_invalid_format_returns_400(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events/GABC123456789012345678901234567890123456789012345678")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "invalid contract_id format");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tx_hash_invalid_length_returns_400(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events/tx/abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "invalid tx_hash format");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tx_hash_non_hex_returns_400(pool: PgPool) {
        let app = create_test_router(pool);
        let invalid_hex = "z".repeat(64);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/events/tx/{}", invalid_hex))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "invalid tx_hash format");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tx_hash_uppercase_hex_returns_400(pool: PgPool) {
        let app = create_test_router(pool);
        let uppercase_hex = "A".repeat(64);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/events/tx/{}", uppercase_hex))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "invalid tx_hash format");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_paginated_returns_approximate_count_by_default(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["approximate"], true);
        assert!(v.get("total").is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_paginated_returns_exact_count_when_requested(pool: PgPool) {
        let app = create_test_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events?exact_count=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["approximate"], false);
        assert_eq!(v["total"], 0); // Empty table
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_with_fields_filter_returns_only_requested_fields(pool: PgPool) {
        let app = create_test_router(pool.clone());
        
        // Insert a test row
        sqlx::query(
            "INSERT INTO events (id, contract_id, event_type, tx_hash, ledger, timestamp, event_data)
             VALUES ($1, $2, $3, $4, $5, $6, $7)"
        )
        .bind(Uuid::new_v4())
        .bind("C1234567890123456789012345678901234567890123456789012345")
        .bind("test")
        .bind("a".repeat(64))
        .bind(100_i64)
        .bind(Utc::now())
        .bind(json!({"foo": "bar"}))
        .execute(&pool)
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/events?fields=id,ledger")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        
        let event = &v["data"][0];
        assert!(event.get("id").is_some());
        assert!(event.get("ledger").is_some());
        assert!(event.get("contract_id").is_none());
        assert!(event.get("event_data").is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_total_count_scenarios(pool: PgPool) {
        let app = create_test_router(pool.clone());

        // 1. Empty set
        let response = app.clone()
            .oneshot(Request::builder().uri("/events").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["total"], 0);
        assert_eq!(v["data"].as_array().unwrap().len(), 0);

        // 2. Single page (3 events, limit 20)
        for i in 0..3 {
            sqlx::query("INSERT INTO events (contract_id, event_type, tx_hash, ledger, timestamp, event_data) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(format!("C{:0>55}", i))
                .bind("contract")
                .bind(format!("{:0>64}", i))
                .bind(i as i64)
                .bind(Utc::now())
                .bind(json!({}))
                .execute(&pool).await.unwrap();
        }

        let response = app.clone()
            .oneshot(Request::builder().uri("/events?limit=20").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v["total"].as_u64().is_some()); // Can be approximate or exact
        assert!(v["total"].as_u64().is_some());
        assert_eq!(v["data"].as_array().unwrap().len(), 3);

        // 3. Multi-page (limit 2, total 3)
        let response = app.clone()
            .oneshot(Request::builder().uri("/events?limit=2&page=1").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v["total"].as_u64().is_some());
        assert_eq!(v["data"].as_array().unwrap().len(), 2);

        let response = app
            .oneshot(Request::builder().uri("/events?limit=2&page=2").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert!(v["total"].as_u64().is_some());
        assert_eq!(v["data"].as_array().unwrap().len(), 1);
    }

    /// Test that health endpoint returns 503 when DB is unreachable
    #[tokio::test]
    async fn health_db_unreachable_returns_503() {
        // Create a pool that will fail to connect
        let pool = PgPool::connect_lazy("postgres://invalid-host:5432/invalid_db").unwrap();
        let health_state = Arc::new(HealthState::new(60));
        let app = crate::routes::create_router(pool, None, &[], 60, health_state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The DB is unreachable so should return 503
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["status"], "degraded");
        assert_eq!(v["db"], "unreachable");
    }
}
