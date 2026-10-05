use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use thiserror::Error;

/// All error variants that any route handler can return.
/// Implements `IntoResponse` so Axum can convert them directly.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    /// Firebase Firestore / Storage errors.  The raw message is logged but
    /// a generic string is returned to the client to avoid leaking internals.
    #[error("Firebase error: {0}")]
    Firebase(String),

    /// Solana RPC errors.
    #[error("Solana error: {0}")]
    Solana(String),

    /// Catch-all for unexpected errors (e.g. from `?` on an `anyhow::Error`).
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, client_message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Firebase(msg) => {
                tracing::error!(firebase_error = %msg, "Firebase service error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Firebase service error".to_owned(),
                )
            }
            AppError::Solana(msg) => {
                tracing::error!(solana_error = %msg, "Solana RPC error");
                (StatusCode::BAD_GATEWAY, "Solana RPC error".to_owned())
            }
            AppError::Internal(e) => {
                tracing::error!(error = ?e, "Internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_owned(),
                )
            }
        };

        (status, Json(serde_json::json!({ "error": client_message }))).into_response()
    }
}

/// Convenience alias used in every handler return type.
pub type ApiResult<T> = Result<T, AppError>;
