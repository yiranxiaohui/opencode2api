use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::Unlocked;
use crate::models::{ClientKeyCreated, ClientKeyInput, ClientKeySummary, OkResponse, now_secs};
use crate::state::AppState;

pub async fn list(
    State(st): State<AppState>,
    _: Unlocked,
) -> Result<Json<Vec<ClientKeySummary>>, ApiError> {
    let keys = st
        .db
        .list_client_keys()?
        .into_iter()
        .map(|row| ClientKeySummary {
            id: row.id,
            name: row.name,
            prefix: row.prefix,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        })
        .collect();
    Ok(Json(keys))
}

pub async fn create(
    State(st): State<AppState>,
    _: Unlocked,
    Json(body): Json<ClientKeyInput>,
) -> Result<Json<ClientKeyCreated>, ApiError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name cannot be empty".into()));
    }
    if name.chars().count() > 80 {
        return Err(ApiError::BadRequest(
            "name must be at most 80 characters".into(),
        ));
    }

    let api_key = crypto::generate_client_key();

    let id = Uuid::new_v4().to_string();
    let created_at = now_secs();
    let prefix = format!("{}…", api_key.chars().take(8).collect::<String>());
    st.db.insert_client_key(
        &id,
        name,
        &crypto::hash_client_key(&api_key),
        &prefix,
        created_at,
    )?;

    Ok(Json(ClientKeyCreated {
        summary: ClientKeySummary {
            id,
            name: name.to_string(),
            prefix,
            created_at,
            last_used_at: None,
        },
        api_key,
    }))
}

pub async fn delete(
    State(st): State<AppState>,
    _: Unlocked,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    if !st.db.delete_client_key(&id)? {
        return Err(ApiError::NotFound("client API key not found".into()));
    }
    Ok(Json(OkResponse { ok: true }))
}
