use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::error::ApiError;
use crate::state::AppState;

/// Extractor that only succeeds when the app is unlocked (master key in memory).
/// Locked requests are rejected with HTTP 423.
pub struct Unlocked;

impl FromRequestParts<AppState> for Unlocked {
    type Rejection = ApiError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.master_key.read().await.is_some() {
            Ok(Unlocked)
        } else {
            Err(ApiError::Locked)
        }
    }
}
