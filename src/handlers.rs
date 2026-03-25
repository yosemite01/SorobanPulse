use axum::{extract::{Path, Query, State}, Json};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{error::AppError, models::{Event, PaginationParams}};

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

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn get_events(
    State(pool): State<PgPool>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, AppError> {
    let limit = params.limit();
    let offset = params.offset();

    let events: Vec<Event> = sqlx::query_as(
        "SELECT *, COUNT(*) OVER () AS total_count FROM events ORDER BY ledger DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await?;

    let total = events.first().map(|e| e.total_count).unwrap_or(0);

    Ok(Json(json!({
        "data": events,
        "total": total,
        "page": params.page.unwrap_or(1),
        "limit": limit
    })))
}

pub async fn get_events_by_contract(
    State(pool): State<PgPool>,
    Path(contract_id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, AppError> {
    validate_contract_id(&contract_id)?;
    
    let limit = params.limit();
    let offset = params.offset();

    let events: Vec<Event> = sqlx::query_as(
        "SELECT * FROM events WHERE contract_id = $1 ORDER BY ledger DESC LIMIT $2 OFFSET $3",
    )
    .bind(&contract_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await?;

    Ok(Json(json!({ "data": events, "contract_id": contract_id })))
}

pub async fn get_events_by_tx(
    State(pool): State<PgPool>,
    Path(tx_hash): Path<String>,
) -> Result<Json<Value>, AppError> {
    validate_tx_hash(&tx_hash)?;
    
    let events: Vec<Event> = sqlx::query_as(
        "SELECT * FROM events WHERE tx_hash = $1 ORDER BY ledger DESC",
    )
    .bind(&tx_hash)
    .fetch_all(&pool)
    .await?;

    Ok(Json(json!({ "data": events, "tx_hash": tx_hash })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[sqlx::test(migrations = "./migrations")]
    async fn get_events_by_tx_no_events_returns_200_empty_data(pool: PgPool) {
        let app = crate::routes::create_router(pool, None, &[], 60);

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
        let app = crate::routes::create_router(pool, None);

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

        let app = crate::routes::create_router(pool, None, &[], 60);

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
        let app = crate::routes::create_router(pool, None, &[], 60);

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
        let app = crate::routes::create_router(pool, None, &[], 60);
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
        let app = crate::routes::create_router(pool, None, &[], 60);

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
        let app = crate::routes::create_router(pool, None, &[], 60);

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
        let app = crate::routes::create_router(pool, None, &[], 60);
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
        let app = crate::routes::create_router(pool, None, &[], 60);
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
    async fn get_events_total_count_scenarios(pool: PgPool) {
        let app = crate::routes::create_router(pool.clone(), None, &[], 60);

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
        assert_eq!(v["total"], 3);
        assert_eq!(v["data"].as_array().unwrap().len(), 3);

        // 3. Multi-page (limit 2, total 3)
        let response = app.clone()
            .oneshot(Request::builder().uri("/events?limit=2&page=1").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["total"], 3);
        assert_eq!(v["data"].as_array().unwrap().len(), 2);

        let response = app
            .oneshot(Request::builder().uri("/events?limit=2&page=2").body(Body::empty()).unwrap())
            .await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["total"], 3);
        assert_eq!(v["data"].as_array().unwrap().len(), 1);
    }
}
