use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    Conflict(String),
    NotFound(String),
    Internal(String),
    ServiceUnavailable(String),
    Upstream(String),
}

impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Upstream(_) => StatusCode::BAD_GATEWAY,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    pub fn message(&self) -> String {
        match self {
            ApiError::Unauthorized(m)
            | ApiError::Forbidden(m)
            | ApiError::BadRequest(m)
            | ApiError::Conflict(m)
            | ApiError::NotFound(m)
            | ApiError::Internal(m)
            | ApiError::ServiceUnavailable(m)
            | ApiError::Upstream(m) => m.clone(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        if matches!(self, ApiError::Internal(_)) {
            tracing::error!("internal error: {}", self.message());
        }
        (status, Json(json!({ "error": self.message() }))).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for ApiError {}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::SqliteFailure(err, msg)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                ApiError::Conflict(msg.unwrap_or_else(|| "constraint violation".into()))
            }
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::Internal(e.to_string())
    }
}
