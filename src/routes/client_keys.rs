use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{
    ClientKeyCreated, ClientKeyInput, ClientKeyModelsInput, ClientKeySummary, OkResponse, now_secs,
};
use crate::state::AppState;

pub async fn list(
    State(st): State<AppState>,
    _: ManagementAuth,
) -> Result<Json<Vec<ClientKeySummary>>, ApiError> {
    let mut keys = Vec::new();
    for row in st.db.list_client_keys()? {
        let api_key = match row.key_enc.as_deref() {
            Some(encrypted) => Some(st.decrypt_secret(encrypted).await?.to_string()),
            None => None,
        };
        keys.push(ClientKeySummary {
            id: row.id,
            name: row.name,
            prefix: row.prefix,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
            api_key,
            allowed_models: row.allowed_models,
        });
    }
    Ok(Json(keys))
}

pub async fn create(
    State(st): State<AppState>,
    _: ManagementAuth,
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
    let allowed_models = normalize_allowed_models(body.allowed_models)?;

    let api_key = crypto::generate_client_key();
    let key_enc = st.encrypt_secret(&api_key).await?;

    let id = Uuid::new_v4().to_string();
    let created_at = now_secs();
    let prefix = format!("{}…", api_key.chars().take(8).collect::<String>());
    let key_hash = crypto::hash_client_key(&api_key);
    st.db.insert_client_key(crate::db::ClientKeyData {
        id: &id,
        name,
        key_hash: &key_hash,
        key_enc: &key_enc,
        prefix: &prefix,
        created_at,
        allowed_models: allowed_models.as_deref(),
    })?;

    Ok(Json(ClientKeyCreated {
        summary: ClientKeySummary {
            id,
            name: name.to_string(),
            prefix,
            created_at,
            last_used_at: None,
            api_key: Some(api_key.clone()),
            allowed_models,
        },
        api_key,
    }))
}

pub async fn update_models(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(body): Json<ClientKeyModelsInput>,
) -> Result<Json<OkResponse>, ApiError> {
    let allowed_models = normalize_allowed_models(body.allowed_models)?;
    if !st
        .db
        .set_client_key_allowed_models(&id, allowed_models.as_deref())?
    {
        return Err(ApiError::NotFound("client API key not found".into()));
    }
    Ok(Json(OkResponse { ok: true }))
}

pub async fn delete(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    if !st.db.delete_client_key(&id)? {
        return Err(ApiError::NotFound("client API key not found".into()));
    }
    Ok(Json(OkResponse { ok: true }))
}

fn normalize_allowed_models(
    allowed_models: Option<Vec<String>>,
) -> Result<Option<Vec<String>>, ApiError> {
    let Some(models) = allowed_models else {
        return Ok(None);
    };
    if models.is_empty() {
        return Err(ApiError::BadRequest(
            "allowed_models must contain at least one model".into(),
        ));
    }
    if models.len() > 256 {
        return Err(ApiError::BadRequest(
            "allowed_models must contain at most 256 models".into(),
        ));
    }

    let mut normalized = Vec::with_capacity(models.len());
    for model in models {
        let model = model.trim();
        if model.is_empty() {
            return Err(ApiError::BadRequest("model ID cannot be empty".into()));
        }
        if model.chars().count() > 200 {
            return Err(ApiError::BadRequest(
                "model ID must be at most 200 characters".into(),
            ));
        }
        if !normalized.iter().any(|existing| existing == model) {
            normalized.push(model.to_string());
        }
    }
    normalized.sort_unstable();
    Ok(Some(normalized))
}

#[cfg(test)]
mod tests {
    use super::normalize_allowed_models;

    #[test]
    fn missing_allowlist_preserves_all_model_access() {
        assert_eq!(normalize_allowed_models(None).unwrap(), None);
    }

    #[test]
    fn allowlist_is_trimmed_deduplicated_and_sorted() {
        let models = normalize_allowed_models(Some(vec![
            " z-model ".into(),
            "a-model".into(),
            "a-model".into(),
        ]))
        .unwrap();
        assert_eq!(models, Some(vec!["a-model".into(), "z-model".into()]));
    }

    #[test]
    fn an_empty_allowlist_is_rejected() {
        assert!(normalize_allowed_models(Some(Vec::new())).is_err());
    }
}
