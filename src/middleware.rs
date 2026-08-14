use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::state::AppState;

/// Extractor that only succeeds while the management user is logged in.
/// Requests made after logout are rejected with HTTP 401.
pub struct Authenticated;

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.master_key.read().await.is_some() {
            Ok(Authenticated)
        } else {
            Err(ApiError::Unauthorized("not logged in".into()))
        }
    }
}
