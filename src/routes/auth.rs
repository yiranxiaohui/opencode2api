use axum::Json;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use uuid::Uuid;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::{WEB_SESSION_COOKIE, WebSession, has_valid_web_session, web_session_token};
use crate::models::{ChangePasswordBody, OkResponse, PasswordBody, StatusResponse};
use crate::state::{AppState, ENCRYPTION_KEY_META};

const WEB_SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;

pub async fn status(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<StatusResponse>, ApiError> {
    let logged_in = has_valid_web_session(&st, &headers)?;
    let installed = st.db.get_meta("password_hash")?.is_some();
    let key_count = st.db.key_count()?;
    Ok(Json(StatusResponse {
        installed,
        logged_in,
        key_count,
    }))
}

/// First-run only: set the login password and start the first session.
pub async fn setup(
    State(st): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<(HeaderMap, Json<OkResponse>), ApiError> {
    if st.db.get_meta("password_hash")?.is_some() {
        return Err(ApiError::Conflict("already installed".into()));
    }
    if body.password.is_empty() {
        return Err(ApiError::BadRequest("password cannot be empty".into()));
    }
    let phc = crypto::hash_password(body.password.as_bytes())?;
    st.db.set_meta("password_hash", &phc)?;
    let key = crypto::derive_key(&phc, body.password.as_bytes())?;
    st.db
        .set_meta(ENCRYPTION_KEY_META, &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    start_web_session(&st)
}

pub async fn login(
    State(st): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<(HeaderMap, Json<OkResponse>), ApiError> {
    let phc = st
        .db
        .get_meta("password_hash")?
        .ok_or_else(|| ApiError::Conflict("not installed".into()))?;
    if !crypto::verify_password(&phc, body.password.as_bytes()) {
        return Err(ApiError::Unauthorized("wrong password".into()));
    }
    let key = crypto::derive_key(&phc, body.password.as_bytes())?;
    st.db
        .set_meta(ENCRYPTION_KEY_META, &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    start_web_session(&st)
}

pub async fn logout(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Json<OkResponse>), ApiError> {
    if let Some(token) = web_session_token(&headers) {
        st.db.delete_web_session(&crypto::hash_client_key(token))?;
    }
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{WEB_SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"
        ))
        .map_err(|error| ApiError::Internal(format!("session cookie: {error}")))?,
    );
    Ok((response_headers, Json(OkResponse { ok: true })))
}

pub async fn change_password(
    State(st): State<AppState>,
    _: WebSession,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<OkResponse>, ApiError> {
    if body.new_password.is_empty() {
        return Err(ApiError::BadRequest("new password cannot be empty".into()));
    }
    let phc = st
        .db
        .get_meta("password_hash")?
        .ok_or_else(|| ApiError::Conflict("not installed".into()))?;
    if !crypto::verify_password(&phc, body.old_password.as_bytes()) {
        return Err(ApiError::Unauthorized("wrong password".into()));
    }
    let old_key = crypto::derive_key(&phc, body.old_password.as_bytes())?;

    let new_phc = crypto::hash_password(body.new_password.as_bytes())?;
    let new_key = crypto::derive_key(&new_phc, body.new_password.as_bytes())?;

    // Re-encrypt every stored key with the new key.
    st.db.reencrypt_all(&old_key, &new_key)?;
    st.db.set_meta("password_hash", &new_phc)?;
    st.db
        .set_meta(ENCRYPTION_KEY_META, &crypto::encode_master_key(&new_key))?;
    *st.master_key.write().await = Some(new_key);
    Ok(Json(OkResponse { ok: true }))
}

pub(crate) fn verify_password(st: &AppState, password: &str) -> Result<(), ApiError> {
    let phc = st
        .db
        .get_meta("password_hash")?
        .ok_or_else(|| ApiError::Conflict("not installed".into()))?;
    if crypto::verify_password(&phc, password.as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::Unauthorized("wrong password".into()))
    }
}

fn start_web_session(st: &AppState) -> Result<(HeaderMap, Json<OkResponse>), ApiError> {
    let token = crypto::generate_web_session_token();
    let now = crate::models::now_secs();
    st.db.insert_web_session(
        &Uuid::new_v4().to_string(),
        &crypto::hash_client_key(&token),
        now,
        now + WEB_SESSION_TTL_SECS,
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{WEB_SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={WEB_SESSION_TTL_SECS}"
        ))
        .map_err(|error| ApiError::Internal(format!("session cookie: {error}")))?,
    );
    Ok((headers, Json(OkResponse { ok: true })))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::http::header::{COOKIE, SET_COOKIE};
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn browser_login_is_cookie_scoped_and_logout_keeps_gateway_secrets_available() {
        let path = std::env::temp_dir().join(format!("opencode2api-auth-{}.db", Uuid::new_v4()));
        crate::migration::run(&path).await.unwrap();
        let state = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();

        let (setup_headers, _) = setup(
            State(state.clone()),
            Json(PasswordBody {
                password: "secret-password".into(),
            }),
        )
        .await
        .unwrap();
        let session_headers = request_headers_from_set_cookie(&setup_headers);
        assert!(state.master_key.read().await.is_some());
        assert!(state.db.get_meta(ENCRYPTION_KEY_META).unwrap().is_some());
        let Json(logged_in) = status(State(state.clone()), session_headers.clone())
            .await
            .unwrap();
        assert!(logged_in.logged_in);
        let Json(no_cookie) = status(State(state.clone()), HeaderMap::new())
            .await
            .unwrap();
        assert!(!no_cookie.logged_in);

        let _ = logout(State(state.clone()), session_headers.clone())
            .await
            .unwrap();
        assert!(state.master_key.read().await.is_some());
        assert!(state.db.get_meta(ENCRYPTION_KEY_META).unwrap().is_some());
        let Json(status_body) = status(State(state.clone()), session_headers).await.unwrap();
        assert!(!status_body.logged_in);

        // The encryption key survives both browser logout and process restart,
        // while the revoked browser session stays logged out.
        let restarted = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();
        assert!(restarted.master_key.read().await.is_some());

        let (login_headers, _) = login(
            State(restarted.clone()),
            Json(PasswordBody {
                password: "secret-password".into(),
            }),
        )
        .await
        .unwrap();
        assert!(restarted.master_key.read().await.is_some());
        let Json(relogged) = status(
            State(restarted.clone()),
            request_headers_from_set_cookie(&login_headers),
        )
        .await
        .unwrap();
        assert!(relogged.logged_in);

        drop(restarted);
        drop(state);
        let _ = std::fs::remove_file(path);
    }

    fn request_headers_from_set_cookie(response_headers: &HeaderMap) -> HeaderMap {
        let cookie = response_headers
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_str(cookie).unwrap());
        headers
    }
}
