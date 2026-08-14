use axum::Json;
use axum::extract::State;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::Authenticated;
use crate::models::{ChangePasswordBody, OkResponse, PasswordBody, StatusResponse};
use crate::state::{AppState, LOGIN_KEY_META};

pub async fn status(State(st): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
    let logged_in = st.master_key.read().await.is_some();
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
) -> Result<Json<OkResponse>, ApiError> {
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
        .set_meta(LOGIN_KEY_META, &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    Ok(Json(OkResponse { ok: true }))
}

pub async fn login(
    State(st): State<AppState>,
    Json(body): Json<PasswordBody>,
) -> Result<Json<OkResponse>, ApiError> {
    let phc = st
        .db
        .get_meta("password_hash")?
        .ok_or_else(|| ApiError::Conflict("not installed".into()))?;
    if !crypto::verify_password(&phc, body.password.as_bytes()) {
        return Err(ApiError::Unauthorized("wrong password".into()));
    }
    let key = crypto::derive_key(&phc, body.password.as_bytes())?;
    st.db
        .set_meta(LOGIN_KEY_META, &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    Ok(Json(OkResponse { ok: true }))
}

pub async fn logout(State(st): State<AppState>) -> Result<Json<OkResponse>, ApiError> {
    // Delete the persisted session first so a database error cannot leave an
    // apparently logged-out process that signs itself back in after restart.
    st.db.delete_meta(LOGIN_KEY_META)?;
    *st.master_key.write().await = None;
    Ok(Json(OkResponse { ok: true }))
}

pub async fn change_password(
    State(st): State<AppState>,
    _: Authenticated,
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
        .set_meta(LOGIN_KEY_META, &crypto::encode_master_key(&new_key))?;
    *st.master_key.write().await = Some(new_key);
    Ok(Json(OkResponse { ok: true }))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn logout_ends_current_and_persisted_login() {
        let path = std::env::temp_dir().join(format!("opencode2api-auth-{}.db", Uuid::new_v4()));
        crate::migration::run(&path).await.unwrap();
        let state = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();

        let _ = setup(
            State(state.clone()),
            Json(PasswordBody {
                password: "secret-password".into(),
            }),
        )
        .await
        .unwrap();
        assert!(state.master_key.read().await.is_some());
        assert!(state.db.get_meta(LOGIN_KEY_META).unwrap().is_some());

        let _ = logout(State(state.clone())).await.unwrap();
        assert!(state.master_key.read().await.is_none());
        assert!(state.db.get_meta(LOGIN_KEY_META).unwrap().is_none());
        let Json(status_body) = status(State(state.clone())).await.unwrap();
        assert!(!status_body.logged_in);

        // Logout is deliberately idempotent, and a restart must not recreate
        // the ended login session.
        let _ = logout(State(state.clone())).await.unwrap();
        let restarted = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();
        assert!(restarted.master_key.read().await.is_none());

        let _ = login(
            State(restarted.clone()),
            Json(PasswordBody {
                password: "secret-password".into(),
            }),
        )
        .await
        .unwrap();
        assert!(restarted.master_key.read().await.is_some());
        assert!(restarted.db.get_meta(LOGIN_KEY_META).unwrap().is_some());

        drop(restarted);
        drop(state);
        let _ = std::fs::remove_file(path);
    }
}
