use std::time::Instant;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::db::KeyData;
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{
    AccountUsage, CookieImportInput, InviteLinkResult, InviteRewardClaimResult,
    InviteRewardsResult, KeyEnabledInput, KeyInput, KeyRecord, KeySummary, ModelInfo,
    OPENCODE_BASE_URL, OkResponse, ProviderRoutingInput, ProviderRoutingStatus, TestResult,
    now_secs,
};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub async fn list(
    State(st): State<AppState>,
    _: ManagementAuth,
    Query(params): Query<ListParams>,
) -> Result<Json<Vec<KeySummary>>, ApiError> {
    let mut items: Vec<KeySummary> = st
        .db
        .list_keys()?
        .into_iter()
        .map(|r| summary(&r))
        .collect();

    if let Some(q) = params.q.filter(|s| !s.trim().is_empty()) {
        let q = q.trim().to_lowercase();
        items.retain(|k| k.name.to_lowercase().contains(&q) || k.notes.to_lowercase().contains(&q));
    }
    if let Some(tag) = params.tag.filter(|s| !s.trim().is_empty()) {
        items.retain(|k| k.tags.iter().any(|t| t == &tag));
    }
    Ok(Json(items))
}

pub async fn get_key(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<KeyRecord>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let api_key = st.decrypt_secret(&row.api_key_enc).await?;
    Ok(Json(KeyRecord {
        summary: summary(&row),
        api_key: api_key.to_string(),
        model_cache: row.model_cache,
    }))
}

pub async fn create(
    State(st): State<AppState>,
    _: ManagementAuth,
    Json(input): Json<KeyInput>,
) -> Result<(StatusCode, Json<KeyRecord>), ApiError> {
    let plain = input
        .api_key
        .ok_or_else(|| ApiError::BadRequest("api_key is required".into()))?;
    if plain.trim().is_empty() {
        return Err(ApiError::BadRequest("api_key cannot be empty".into()));
    }
    let plain = plain.trim().to_string();
    let mut name = if input.name.trim().is_empty() {
        let suffix: String = plain
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("OpenCode {suffix}")
    } else {
        input.name.trim().to_string()
    };
    if st.db.get_key_by_name(&name)?.is_some() {
        name = format!("{name} {}", &Uuid::new_v4().simple().to_string()[..4]);
    }
    let proxy_id = resolve_proxy(&st, input.proxy_id.flatten().as_deref()).await?;

    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    let enc = st.encrypt_secret(&plain).await?;
    st.db.insert_key(
        &id,
        &KeyData {
            name,
            base_url: OPENCODE_BASE_URL.to_string(),
            api_key_enc: enc,
            tags: input.tags,
            notes: input.notes,
            is_default: false,
            is_enabled: true,
            account_type: input.account_type.unwrap_or_default(),
            proxy_id,
            cookie_enc: None,
            workspace_id: None,
        },
        now,
    )?;

    let row = st.db.get_key(&id)?.unwrap();
    let decrypted = st.decrypt_secret(&row.api_key_enc).await?;
    Ok((
        StatusCode::CREATED,
        Json(KeyRecord {
            summary: summary(&row),
            api_key: decrypted.to_string(),
            model_cache: row.model_cache,
        }),
    ))
}

pub async fn update(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<KeyInput>,
) -> Result<Json<KeyRecord>, ApiError> {
    let existing = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;

    let name = if input.name.trim().is_empty() {
        existing.name.clone()
    } else {
        input.name.trim().to_string()
    };
    if existing.name.to_lowercase() != name.to_lowercase() {
        if st.db.get_key_by_name(&name)?.is_some() {
            return Err(ApiError::Conflict(format!("name already exists: {name}")));
        }
    }

    let api_key_replaced = input
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty());
    let enc = match &input.api_key {
        Some(k) if !k.trim().is_empty() => st.encrypt_secret(k).await?,
        _ => existing.api_key_enc.clone(),
    };
    let proxy_id = match input.proxy_id {
        Some(value) => resolve_proxy(&st, value.as_deref()).await?,
        None => existing.proxy_id.clone(),
    };

    st.db.update_key(
        &id,
        &KeyData {
            name,
            base_url: OPENCODE_BASE_URL.to_string(),
            api_key_enc: enc,
            tags: input.tags,
            notes: input.notes,
            is_default: false,
            is_enabled: existing.is_enabled,
            account_type: input.account_type.unwrap_or(existing.account_type),
            proxy_id,
            cookie_enc: existing.cookie_enc.clone(),
            workspace_id: existing.workspace_id.clone(),
        },
        now_secs(),
    )?;
    if api_key_replaced {
        st.db.set_quota_exhausted(&id, false, now_secs())?;
    }

    let row = st.db.get_key(&id)?.unwrap();
    let decrypted = st.decrypt_secret(&row.api_key_enc).await?;
    Ok(Json(KeyRecord {
        summary: summary(&row),
        api_key: decrypted.to_string(),
        model_cache: row.model_cache,
    }))
}

pub async fn import_cookie(
    State(st): State<AppState>,
    _: ManagementAuth,
    Json(input): Json<CookieImportInput>,
) -> Result<(StatusCode, Json<KeyRecord>), ApiError> {
    let record = import_cookie_record(&st, input).await?;
    Ok((StatusCode::CREATED, Json(record)))
}

pub(crate) async fn import_cookie_record(
    st: &AppState,
    input: CookieImportInput,
) -> Result<KeyRecord, ApiError> {
    let cookie = crate::opencode_account::normalize_cookie(&input.cookie)?;
    let proxy_id = resolve_proxy(st, input.proxy_id.as_deref()).await?;
    let client = st.client_for_proxy_id(proxy_id.as_deref()).await?;
    let (workspace_id, api_key, email) =
        crate::opencode_account::discover(&client, &cookie).await?;
    let mut name = input
        .name
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .or(email)
        .unwrap_or_else(|| format!("OpenCode {workspace_id}"));
    if st.db.get_key_by_name(&name)?.is_some() {
        name = format!("{name} {}", &Uuid::new_v4().simple().to_string()[..4]);
    }
    let id = Uuid::new_v4().to_string();
    st.db.insert_key(
        &id,
        &KeyData {
            name,
            base_url: OPENCODE_BASE_URL.into(),
            api_key_enc: st.encrypt_secret(&api_key).await?,
            tags: vec![],
            notes: "Cookie 导入".into(),
            is_default: false,
            is_enabled: true,
            account_type: input.account_type,
            proxy_id,
            cookie_enc: Some(st.encrypt_secret(&cookie).await?),
            workspace_id: Some(workspace_id),
        },
        now_secs(),
    )?;
    let row = st.db.get_key(&id)?.unwrap();
    Ok(KeyRecord {
        summary: summary(&row),
        api_key,
        model_cache: vec![],
    })
}

fn summary(row: &crate::db::KeyRow) -> KeySummary {
    KeySummary::from_row(row)
}

pub async fn usage(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<AccountUsage>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let usage = crate::account_monitor::refresh_account_usage(&st, &row).await?;
    Ok(Json(usage))
}

pub async fn provider_routing(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<ProviderRoutingStatus>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let (Some(cookie_enc), Some(workspace)) =
        (row.cookie_enc.as_deref(), row.workspace_id.as_deref())
    else {
        return Ok(Json(ProviderRoutingStatus {
            supported: false,
            enabled: None,
            reason: Some("仅通过 Cookie 导入的 OpenCode Go 账号支持此设置".into()),
        }));
    };
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(&row).await?;
    let enabled = crate::opencode_account::provider_routing(&client, &cookie, workspace).await?;
    Ok(Json(match enabled {
        Some(enabled) => ProviderRoutingStatus {
            supported: true,
            enabled: Some(enabled),
            reason: None,
        },
        None => ProviderRoutingStatus {
            supported: false,
            enabled: None,
            reason: Some("该账号当前没有可管理的 OpenCode Go 订阅".into()),
        },
    }))
}

pub async fn set_provider_routing(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<ProviderRoutingInput>,
) -> Result<Json<ProviderRoutingStatus>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let cookie_enc = row.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("仅通过 Cookie 导入的 OpenCode Go 账号支持此设置".into())
    })?;
    let workspace = row
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::Internal("账号缺少 workspace".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(&row).await?;
    let enabled =
        crate::opencode_account::set_provider_routing(&client, &cookie, workspace, input.enabled)
            .await?;
    Ok(Json(ProviderRoutingStatus {
        supported: true,
        enabled: Some(enabled),
        reason: None,
    }))
}

pub async fn get_invite_link(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<InviteLinkResult>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let cookie_enc = row.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("该账号不是通过 Cookie 导入，无法获取邀请链接".into())
    })?;
    let workspace = row
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::Internal("账号缺少 workspace".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(&row).await?;
    let invite_link = crate::opencode_account::invite_link(&client, &cookie, workspace).await?;
    Ok(Json(InviteLinkResult {
        account_id: row.id,
        account_name: row.name,
        invite_link,
    }))
}

pub async fn invite_rewards(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<InviteRewardsResult>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let cookie_enc = row.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("该账号不是通过 Cookie 导入，无法查询邀请奖励".into())
    })?;
    let workspace = row
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::Internal("账号缺少 workspace".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(&row).await?;
    let rewards = crate::opencode_account::invite_rewards(&client, &cookie, workspace).await?;
    Ok(Json(InviteRewardsResult {
        account_id: row.id,
        account_name: row.name,
        rewards,
    }))
}

pub async fn claim_invite_reward(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path((id, reward_id)): Path<(String, String)>,
) -> Result<Json<InviteRewardClaimResult>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let cookie_enc = row.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("该账号不是通过 Cookie 导入，无法领取邀请奖励".into())
    })?;
    let workspace = row
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::Internal("账号缺少 workspace".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(&row).await?;
    let amount_cents =
        crate::opencode_account::claim_invite_reward(&client, &cookie, workspace, &reward_id)
            .await?;
    Ok(Json(InviteRewardClaimResult {
        account_id: row.id,
        account_name: row.name,
        reward_id,
        amount_cents,
    }))
}

pub async fn delete(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    st.db
        .delete_key(&id)?
        .then(|| Json(OkResponse { ok: true }))
        .ok_or_else(|| ApiError::NotFound("key not found".into()))
}

pub async fn set_enabled(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<KeyEnabledInput>,
) -> Result<Json<OkResponse>, ApiError> {
    st.db
        .set_key_enabled(&id, input.enabled, now_secs())?
        .then_some(Json(OkResponse { ok: true }))
        .ok_or_else(|| ApiError::NotFound("key not found".into()))
}

pub async fn test(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<TestResult>, ApiError> {
    let row = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("key not found".into()))?;
    let api_key = st.decrypt_secret(&row.api_key_enc).await?;
    let url = format!("{OPENCODE_BASE_URL}/models");

    // Connectivity tests route through the key's attached proxy (if any), with
    // the same short timeout as the plain test client.
    let client = match &row.proxy_id {
        Some(proxy_id) => {
            let proxy = st
                .db
                .get_proxy(proxy_id)?
                .ok_or_else(|| ApiError::Internal("attached proxy not found".into()))?;
            let proxy_url = st.decrypt_secret(&proxy.url_enc).await?;
            crate::state::build_proxy_client(&proxy_url, std::time::Duration::from_secs(10))?
        }
        None => st.test_client.clone(),
    };

    let start = Instant::now();
    let resp = client.get(&url).bearer_auth(api_key.as_str()).send().await;
    let latency_ms = start.elapsed().as_millis();

    match resp {
        Ok(r) if r.status().is_success() => {
            let json: serde_json::Value = r
                .json()
                .await
                .map_err(|e| ApiError::Internal(format!("bad JSON from upstream: {e}")))?;
            let models: Vec<ModelInfo> = json
                .get("data")
                .and_then(|d| d.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let owned_by = m
                                .get("owned_by")
                                .and_then(|o| o.as_str())
                                .unwrap_or("")
                                .to_string();
                            Some(ModelInfo { id, owned_by })
                        })
                        .collect()
                })
                .unwrap_or_default();
            st.db.set_model_cache(&id, &models, now_secs())?;
            Ok(Json(TestResult {
                ok: true,
                latency_ms: Some(latency_ms),
                models,
                error: None,
            }))
        }
        Ok(r) => {
            let status = r.status();
            let err = truncate(&r.text().await.unwrap_or_default(), 300);
            Ok(Json(TestResult {
                ok: false,
                latency_ms: Some(latency_ms),
                models: vec![],
                error: Some(format!("HTTP {status}: {err}")),
            }))
        }
        Err(e) => Ok(Json(TestResult {
            ok: false,
            latency_ms: Some(latency_ms),
            models: vec![],
            error: Some(e.to_string()),
        })),
    }
}

/// Validate a requested proxy id (if any) and return it, or `None` for no proxy.
pub(crate) async fn resolve_proxy(
    st: &AppState,
    proxy_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(pid) = proxy_id.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    if st.db.get_proxy(pid)?.is_none() {
        return Err(ApiError::BadRequest(format!("proxy not found: {pid}")));
    }
    Ok(Some(pid.to_string()))
}
