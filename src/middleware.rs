use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    pub api_key: Option<String>,
}

pub async fn auth_middleware(
    State(state): State<Arc<AuthState>>,
    req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let path = req.uri().path();
    
    // Exclude /health and /healthz/*
    if path == "/health" || path.starts_with("/healthz/") {
        return Ok(next.run(req).await);
    }
    
    if let Some(expected_key) = &state.api_key {
        let auth_header = req.headers().get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
            
        let api_key_header = req.headers().get("X-Api-Key")
            .and_then(|h| h.to_str().ok());
            
        let provided_key = auth_header.or(api_key_header);
        
        if provided_key != Some(expected_key.as_str()) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unauthorized" }))
            ));
        }
    }
    
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use tower::ServiceExt;
    use axum::http::{Request, StatusCode};
    use axum::body::Body;
    use axum::response::Response;

    async fn setup_app(api_key: Option<String>) -> Router {
        let auth_state = Arc::new(AuthState { api_key });
        Router::new()
            .route("/test", get(|| async { "OK" }))
            .route("/health", get(|| async { "OK" }))
            .route("/healthz/live", get(|| async { "OK" }))
            .route_layer(axum::middleware::from_fn_with_state(auth_state, auth_middleware))
    }

    #[tokio::test]
    async fn test_auth_bypassed_when_no_key_configured() {
        let app = setup_app(None).await;
        
        let response: Response = app
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();
            
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_success_with_bearer_token() {
        let app = setup_app(Some("secret123".to_string())).await;
        
        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Bearer secret123")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
            
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_success_with_x_api_key() {
        let app = setup_app(Some("secret123".to_string())).await;
        
        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("X-Api-Key", "secret123")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
            
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_failure_with_invalid_key() {
        let app = setup_app(Some("secret123".to_string())).await;
        
        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("Authorization", "Bearer wrongkey")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
            
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_failure_with_missing_key() {
        let app = setup_app(Some("secret123".to_string())).await;
        
        let response: Response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
            
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_health_endpoints_bypass_auth() {
        let app = setup_app(Some("secret123".to_string())).await;
        
        let response: Response = app.clone()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        
        let response: Response = app
            .oneshot(Request::builder().uri("/healthz/live").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
