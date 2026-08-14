use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::crypto;
use crate::error::ApiError;
use crate::middleware::{ADMIN_READ_SCOPE, ADMIN_WRITE_SCOPE, WebSession};
use crate::models::{
    AdminTokenCreated, AdminTokenInput, AdminTokenSummary, OkResponse, RevokeAdminTokenInput,
    now_secs,
};
use crate::state::AppState;

pub async fn list(
    State(st): State<AppState>,
    _: WebSession,
) -> Result<Json<Vec<AdminTokenSummary>>, ApiError> {
    Ok(Json(
        st.db
            .list_admin_tokens()?
            .into_iter()
            .map(|row| AdminTokenSummary {
                id: row.id,
                name: row.name,
                prefix: row.prefix,
                scopes: row.scopes,
                created_at: row.created_at,
                last_used_at: row.last_used_at,
            })
            .collect(),
    ))
}

pub async fn create(
    State(st): State<AppState>,
    _: WebSession,
    Json(body): Json<AdminTokenInput>,
) -> Result<Json<AdminTokenCreated>, ApiError> {
    super::auth::verify_password(&st, &body.password)?;
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name cannot be empty".into()));
    }
    if name.chars().count() > 80 {
        return Err(ApiError::BadRequest(
            "name must be at most 80 characters".into(),
        ));
    }
    let scopes = normalize_scopes(&body.scopes)?;

    let token = crypto::generate_admin_token();
    let id = Uuid::new_v4().to_string();
    let created_at = now_secs();
    let prefix = format!("{}…", token.chars().take(18).collect::<String>());
    st.db.insert_admin_token(
        &id,
        name,
        &crypto::hash_client_key(&token),
        &prefix,
        &scopes,
        created_at,
    )?;

    Ok(Json(AdminTokenCreated {
        summary: AdminTokenSummary {
            id,
            name: name.to_string(),
            prefix,
            scopes,
            created_at,
            last_used_at: None,
        },
        token,
    }))
}

pub async fn revoke(
    State(st): State<AppState>,
    _: WebSession,
    Path(id): Path<String>,
    Json(body): Json<RevokeAdminTokenInput>,
) -> Result<Json<OkResponse>, ApiError> {
    super::auth::verify_password(&st, &body.password)?;
    if !st.db.delete_admin_token(&id)? {
        return Err(ApiError::NotFound("management token not found".into()));
    }
    Ok(Json(OkResponse { ok: true }))
}

fn normalize_scopes(input: &[String]) -> Result<Vec<String>, ApiError> {
    if let Some(scope) = input
        .iter()
        .find(|scope| scope.as_str() != ADMIN_READ_SCOPE && scope.as_str() != ADMIN_WRITE_SCOPE)
    {
        return Err(ApiError::BadRequest(format!(
            "unsupported management token scope: {scope}"
        )));
    }
    let scopes = [ADMIN_READ_SCOPE, ADMIN_WRITE_SCOPE]
        .into_iter()
        .filter(|allowed| input.iter().any(|scope| scope == allowed))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if scopes.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one management token scope is required".into(),
        ));
    }
    Ok(scopes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_are_validated_deduplicated_and_canonicalized() {
        let scopes = normalize_scopes(&[
            ADMIN_WRITE_SCOPE.into(),
            ADMIN_READ_SCOPE.into(),
            ADMIN_WRITE_SCOPE.into(),
        ])
        .unwrap();
        assert_eq!(scopes, vec![ADMIN_READ_SCOPE, ADMIN_WRITE_SCOPE]);
        assert!(normalize_scopes(&["unknown".into()]).is_err());
        assert!(normalize_scopes(&[]).is_err());
    }
}
