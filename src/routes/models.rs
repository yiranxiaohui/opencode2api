use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::middleware::Authenticated;
use crate::models::{ManagedModel, ModelEnabledInput, OkResponse};
use crate::state::AppState;

/// Gateway-wide model catalog. Every configured route is queried concurrently
/// and duplicate model IDs are collapsed into one OpenAI-compatible entry.
pub async fn models(State(st): State<AppState>, _: Authenticated, headers: HeaderMap) -> Response {
    match models_inner(st, &headers).await {
        Ok(value) => Json(value).into_response(),
        Err(error) => super::proxy::openai_error(error),
    }
}

async fn models_inner(st: AppState, headers: &HeaderMap) -> Result<Value, ApiError> {
    let _client = match super::proxy::authenticate_client(&st, headers) {
        Ok(client) => client,
        Err(error) => {
            super::logs::record_auth_failure(&st, "GET", "/models", &error);
            return Err(error);
        }
    };
    let routes: Vec<_> = st
        .db
        .list_keys()?
        .into_iter()
        .filter(|route| route.is_enabled)
        .collect();
    if routes.is_empty() {
        return Err(ApiError::BadRequest("no upstream routes configured".into()));
    }

    let results = stream::iter(routes.into_iter().map(|route| {
        let st = st.clone();
        async move {
            let api_key = st.decrypt_secret(&route.api_key_enc).await?;
            let url = format!("{}/models", crate::models::OPENCODE_BASE_URL);
            // Route through the key's attached proxy (if any) with a short timeout.
            let client = match &route.proxy_id {
                Some(proxy_id) => {
                    let proxy = st
                        .db
                        .get_proxy(proxy_id)?
                        .ok_or_else(|| ApiError::Internal("attached proxy not found".into()))?;
                    let proxy_url = st.decrypt_secret(&proxy.url_enc).await?;
                    crate::state::build_proxy_client(
                        &proxy_url,
                        std::time::Duration::from_secs(10),
                    )?
                }
                None => st.test_client.clone(),
            };
            let response = client
                .get(url)
                .bearer_auth(api_key.as_str())
                .send()
                .await
                .map_err(|e| ApiError::Upstream(format!("{}: {e}", route.name)))?;
            if !response.status().is_success() {
                return Err(ApiError::Upstream(format!(
                    "{} returned HTTP {}",
                    route.name,
                    response.status()
                )));
            }
            let value = response.json::<Value>().await.map_err(|e| {
                ApiError::Upstream(format!("{}: invalid model response: {e}", route.name))
            })?;
            Ok::<_, ApiError>((route.name, value))
        }
    }))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;

    let mut models = BTreeMap::<String, Value>::new();
    let mut failures = Vec::new();
    let mut successful_routes = 0;
    for result in results {
        match result {
            Ok((_route_name, value)) => {
                successful_routes += 1;
                if let Some(items) = value.get("data").and_then(Value::as_array) {
                    for item in items {
                        if let Some(id) = item.get("id").and_then(Value::as_str) {
                            let normalized = if item.is_object() {
                                item.clone()
                            } else {
                                json!({"id": id, "object": "model", "owned_by": "unknown"})
                            };
                            models.entry(id.to_string()).or_insert(normalized);
                        }
                    }
                }
            }
            Err(error) => failures.push(error.message()),
        }
    }

    if successful_routes == 0 {
        return Err(ApiError::Upstream(format!(
            "failed to load models from every upstream: {}",
            failures.join("; ")
        )));
    }

    let disabled = st.db.disabled_models()?;
    Ok(json!({
        "object": "list",
        "data": models.into_iter().filter(|(id, _)| !disabled.contains(id)).map(|(_, value)| value).collect::<Vec<_>>()
    }))
}

pub async fn catalog(
    State(st): State<AppState>,
    _: Authenticated,
) -> Result<Json<Vec<ManagedModel>>, ApiError> {
    let disabled = st.db.disabled_models()?;
    let mut models = BTreeMap::<String, (String, usize)>::new();
    for account in st
        .db
        .list_keys()?
        .into_iter()
        .filter(|item| item.is_enabled)
    {
        for model in account.model_cache {
            let entry = models.entry(model.id).or_insert((model.owned_by, 0));
            entry.1 += 1;
        }
    }
    Ok(Json(
        models
            .into_iter()
            .map(|(id, (owned_by, account_count))| ManagedModel {
                enabled: !disabled.contains(&id),
                id,
                owned_by,
                account_count,
            })
            .collect(),
    ))
}

pub async fn set_enabled(
    State(st): State<AppState>,
    _: Authenticated,
    Json(input): Json<ModelEnabledInput>,
) -> Result<Json<OkResponse>, ApiError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err(ApiError::BadRequest("模型 ID 不能为空".into()));
    }
    st.db.set_model_enabled(id, input.enabled)?;
    Ok(Json(OkResponse { ok: true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_are_deduplicated() {
        let mut models = BTreeMap::new();
        for item in [
            json!({"id":"gpt-4o"}),
            json!({"id":"gpt-4o"}),
            json!({"id":"gpt-4.1"}),
        ] {
            let id = item["id"].as_str().unwrap().to_string();
            models.entry(id).or_insert(item);
        }
        assert_eq!(models.len(), 2);
    }
}
