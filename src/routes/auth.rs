use axum::Json;
use axum::extract::State;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::Unlocked;
use crate::models::{ChangePasswordBody, OkResponse, PasswordBody, StatusResponse};
use crate::state::AppState;

pub async fn status(State(st): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
    let unlocked = st.master_key.read().await.is_some();
    let installed = st.db.get_meta("password_hash")?.is_some();
    let key_count = st.db.key_count()?;
    Ok(Json(StatusResponse {
        installed,
        unlocked,
        key_count,
    }))
}

/// First-run only: set the master password and unlock immediately.
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
        .set_meta("auto_unlock_key", &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    Ok(Json(OkResponse { ok: true }))
}

pub async fn unlock(
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
        .set_meta("auto_unlock_key", &crypto::encode_master_key(&key))?;
    *st.master_key.write().await = Some(key);
    Ok(Json(OkResponse { ok: true }))
}

pub async fn change_password(
    State(st): State<AppState>,
    _: Unlocked,
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
        .set_meta("auto_unlock_key", &crypto::encode_master_key(&new_key))?;
    *st.master_key.write().await = Some(new_key);
    Ok(Json(OkResponse { ok: true }))
}
