use thiserror::Error;
use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Internal error: {0}")]
    #[allow(dead_code)]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let correlation_id = Uuid::new_v4().to_string();
        
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Database(e) => {
                tracing::error!(
                    correlation_id = %correlation_id,
                    error = %e,
                    "Database error"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Http(e) => {
                tracing::error!(
                    correlation_id = %correlation_id,
                    error = %e,
                    "HTTP error"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Internal(msg) => {
                tracing::error!(
                    correlation_id = %correlation_id,
                    error = %msg,
                    "Internal error"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };
        
        (status, Json(json!({ "error": message }))).into_response()
    }
}
