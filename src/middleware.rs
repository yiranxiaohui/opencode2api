use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};

use crate::crypto;
use crate::error::ApiError;
use crate::models::now_secs;
use crate::state::AppState;

pub const ADMIN_READ_SCOPE: &str = "admin:read";
pub const ADMIN_WRITE_SCOPE: &str = "admin:write";
pub const WEB_SESSION_COOKIE: &str = "opencode2api_session";

/// Browser-only authentication. This deliberately does not accept management
/// Bearer tokens and protects password and token lifecycle operations.
pub struct WebSession;

/// Authentication for management APIs. A request may use either a valid
/// browser session cookie or an independent scoped Bearer token.
pub struct ManagementAuth;

/// Gateway endpoints authenticate their own client API keys. They only need
/// the encryption key to be available and do not depend on a browser login.
pub struct SecretsAvailable;

impl FromRequestParts<AppState> for WebSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !has_valid_web_session(state, &parts.headers)? {
            return Err(ApiError::Unauthorized("web login required".into()));
        }
        ensure_secrets_available(state).await?;
        Ok(WebSession)
    }
}

impl FromRequestParts<AppState> for ManagementAuth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if let Some(value) = parts.headers.get(AUTHORIZATION) {
            let value = value
                .to_str()
                .map_err(|_| ApiError::Unauthorized("invalid Authorization header".into()))?;
            let token = parse_bearer(value)
                .ok_or_else(|| ApiError::Unauthorized("Bearer management token required".into()))?;
            let token_hash = crypto::hash_client_key(token);
            let row = state
                .db
                .admin_token_by_hash(&token_hash)?
                .ok_or_else(|| ApiError::Unauthorized("invalid management token".into()))?;
            let required_scope = required_scope(&parts.method);
            if !row.scopes.iter().any(|scope| scope == required_scope) {
                return Err(ApiError::Forbidden(format!(
                    "management token requires {required_scope}"
                )));
            }
            state.db.touch_admin_token(&row.id, now_secs())?;
        } else if !has_valid_web_session(state, &parts.headers)? {
            return Err(ApiError::Unauthorized(
                "web login or Bearer management token required".into(),
            ));
        }

        ensure_secrets_available(state).await?;
        Ok(ManagementAuth)
    }
}

impl FromRequestParts<AppState> for SecretsAvailable {
    type Rejection = ApiError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        ensure_secrets_available(state).await?;
        Ok(SecretsAvailable)
    }
}

pub fn web_session_token(headers: &HeaderMap) -> Option<&str> {
    headers.get_all(COOKIE).iter().find_map(|header| {
        header.to_str().ok()?.split(';').find_map(|cookie| {
            let (name, value) = cookie.trim().split_once('=')?;
            (name == WEB_SESSION_COOKIE && !value.is_empty()).then_some(value)
        })
    })
}

pub fn has_valid_web_session(state: &AppState, headers: &HeaderMap) -> Result<bool, ApiError> {
    let Some(token) = web_session_token(headers) else {
        return Ok(false);
    };
    state
        .db
        .authenticate_web_session(&crypto::hash_client_key(token), now_secs())
}

fn parse_bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

fn required_scope(method: &Method) -> &'static str {
    if matches!(*method, Method::GET | Method::HEAD) {
        ADMIN_READ_SCOPE
    } else {
        ADMIN_WRITE_SCOPE
    }
}

async fn ensure_secrets_available(state: &AppState) -> Result<(), ApiError> {
    if state.master_key.read().await.is_some() {
        Ok(())
    } else {
        Err(ApiError::ServiceUnavailable(
            "encryption key unavailable; log in once to restore service".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::http::Request;
    use uuid::Uuid;
    use zeroize::Zeroizing;

    use super::*;

    #[tokio::test]
    async fn management_tokens_enforce_scopes_and_browser_sessions_are_independent() {
        let path = std::env::temp_dir().join(format!("opencode2api-authn-{}.db", Uuid::new_v4()));
        crate::migration::run(&path).await.unwrap();
        let state = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();
        *state.master_key.write().await = Some(Zeroizing::new([7; crate::crypto::KEY_LEN]));

        let read_token = crypto::generate_admin_token();
        state
            .db
            .insert_admin_token(
                "read-token",
                "Read only",
                &crypto::hash_client_key(&read_token),
                "oca_admin_test…",
                &[ADMIN_READ_SCOPE.into()],
                now_secs(),
            )
            .unwrap();

        let mut read_parts = Request::builder()
            .method(Method::GET)
            .header(AUTHORIZATION, format!("Bearer {read_token}"))
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert!(
            ManagementAuth::from_request_parts(&mut read_parts, &state)
                .await
                .is_ok()
        );
        assert!(
            state.db.list_admin_tokens().unwrap()[0]
                .last_used_at
                .is_some()
        );

        let mut write_parts = Request::builder()
            .method(Method::POST)
            .header(AUTHORIZATION, format!("Bearer {read_token}"))
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert!(matches!(
            ManagementAuth::from_request_parts(&mut write_parts, &state).await,
            Err(ApiError::Forbidden(_))
        ));

        let write_token = crypto::generate_admin_token();
        state
            .db
            .insert_admin_token(
                "write-token",
                "Write only",
                &crypto::hash_client_key(&write_token),
                "oca_admin_write…",
                &[ADMIN_WRITE_SCOPE.into()],
                now_secs(),
            )
            .unwrap();
        let mut allowed_write_parts = Request::builder()
            .method(Method::DELETE)
            .header(AUTHORIZATION, format!("Bearer {write_token}"))
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert!(
            ManagementAuth::from_request_parts(&mut allowed_write_parts, &state)
                .await
                .is_ok()
        );

        let session_token = crypto::generate_web_session_token();
        let now = now_secs();
        state
            .db
            .insert_web_session(
                "browser-session",
                &crypto::hash_client_key(&session_token),
                now,
                now + 60,
            )
            .unwrap();
        let mut browser_parts = Request::builder()
            .method(Method::POST)
            .header(COOKIE, format!("{WEB_SESSION_COOKIE}={session_token}"))
            .body(())
            .unwrap()
            .into_parts()
            .0;
        assert!(
            ManagementAuth::from_request_parts(&mut browser_parts, &state)
                .await
                .is_ok()
        );

        drop(state);
        let _ = std::fs::remove_file(path);
    }
}
