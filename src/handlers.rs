use axum::{extract::{Path, Query, State}, Json};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::{error::AppError, models::{Event, PaginationParams}};

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
        "SELECT * FROM events ORDER BY ledger DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&pool)
        .await?;

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

    if events.is_empty() {
        return Err(AppError::NotFound);
    }

    Ok(Json(json!({ "data": events, "contract_id": contract_id })))
}

pub async fn get_events_by_tx(
    State(pool): State<PgPool>,
    Path(tx_hash): Path<String>,
) -> Result<Json<Value>, AppError> {
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
        let app = crate::routes::create_router(pool, None);

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

        let app = crate::routes::create_router(pool, None);

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
}
