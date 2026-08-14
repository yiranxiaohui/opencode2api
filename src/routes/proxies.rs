use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::db::ProxyData;
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{OkResponse, ProxyInput, ProxyRecord, now_secs};
use crate::state::AppState;

fn row_to_record(r: &crate::db::ProxyRow, url: String) -> ProxyRecord {
    ProxyRecord {
        id: r.id.clone(),
        name: r.name.clone(),
        url,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

pub async fn list(
    State(st): State<AppState>,
    _: ManagementAuth,
) -> Result<Json<Vec<ProxyRecord>>, ApiError> {
    let rows = st.db.list_proxies()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let url = st.decrypt_secret(&row.url_enc).await?.to_string();
        out.push(row_to_record(&row, url));
    }
    Ok(Json(out))
}

pub async fn create(
    State(st): State<AppState>,
    _: ManagementAuth,
    Json(input): Json<ProxyInput>,
) -> Result<(StatusCode, Json<ProxyRecord>), ApiError> {
    let name = input.name.trim().to_string();
    let url = input.url.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if st.db.get_proxy_by_name(&name)?.is_some() {
        return Err(ApiError::Conflict(format!("name already exists: {name}")));
    }
    // Validate the scheme early so a typo surfaces here, not at request time.
    crate::state::build_proxy_client(&url, std::time::Duration::from_secs(5))?;

    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    let url_enc = st.encrypt_secret(&url).await?;
    st.db.insert_proxy(&id, &ProxyData { name, url_enc }, now)?;
    st.clear_proxy_client_cache();

    let row = st.db.get_proxy(&id)?.unwrap();
    let decrypted = st.decrypt_secret(&row.url_enc).await?.to_string();
    Ok((StatusCode::CREATED, Json(row_to_record(&row, decrypted))))
}

pub async fn update(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<ProxyInput>,
) -> Result<Json<ProxyRecord>, ApiError> {
    let existing = st
        .db
        .get_proxy(&id)?
        .ok_or_else(|| ApiError::NotFound("proxy not found".into()))?;
    let name = input.name.trim().to_string();
    let url = input.url.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if existing.name.to_lowercase() != name.to_lowercase() {
        if st.db.get_proxy_by_name(&name)?.is_some() {
            return Err(ApiError::Conflict(format!("name already exists: {name}")));
        }
    }
    crate::state::build_proxy_client(&url, std::time::Duration::from_secs(5))?;

    let url_enc = st.encrypt_secret(&url).await?;
    let row = st
        .db
        .update_proxy(&id, &ProxyData { name, url_enc }, now_secs())?
        .ok_or_else(|| ApiError::NotFound("proxy not found".into()))?;
    st.clear_proxy_client_cache();

    let decrypted = st.decrypt_secret(&row.url_enc).await?.to_string();
    Ok(Json(row_to_record(&row, decrypted)))
}

pub async fn delete(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    if !st.db.delete_proxy(&id)? {
        return Err(ApiError::NotFound("proxy not found".into()));
    }
    st.clear_proxy_client_cache();
    Ok(Json(OkResponse { ok: true }))
}
