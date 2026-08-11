use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::middleware::Unlocked;
use crate::routes::logs::{self, LogInput};
use crate::state::AppState;

/// Unified OpenAI-compatible gateway: forwards `/v1/*` to a round-robin key.
/// `X-Key-Id` / `X-Key-Name` remain available as explicit overrides.
/// SSE (stream: true) is forwarded incrementally via `Body::from_stream`.
pub async fn proxy(
    State(st): State<AppState>,
    _: Unlocked,
    method: Method,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    match proxy_inner(st, method, &path, &headers, body, None).await {
        Ok(resp) => resp,
        Err(e) => openai_error(e),
    }
}

pub async fn chat_proxy(
    State(st): State<AppState>,
    _: Unlocked,
    method: Method,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let client = crate::db::ClientKeyRow {
        id: "chat".into(),
        name: "内置对话".into(),
        prefix: "internal".into(),
        created_at: 0,
        last_used_at: None,
    };
    match proxy_inner(st, method, &path, &headers, body, Some(client)).await {
        Ok(resp) => resp,
        Err(e) => openai_error(e),
    }
}

pub(crate) async fn proxy_inner(
    st: AppState,
    method: Method,
    path: &str,
    headers: &HeaderMap,
    body: Body,
    authenticated_client: Option<crate::db::ClientKeyRow>,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    // Copy the method name before `method` is moved into the upstream request.
    let method_str = method.as_str().to_string();
    let client = match authenticated_client {
        Some(client) => client,
        None => match authenticate_client(&st, headers) {
            Ok(client) => client,
            Err(error) => {
                logs::record_auth_failure(&st, &method_str, path, &error);
                return Err(error);
            }
        },
    };
    // Requests are small JSON bodies; buffer fully before routing so a standard
    // client can be routed by its `model` without custom headers.
    let body_bytes = body
        .collect()
        .await
        .map_err(|e| ApiError::Internal(format!("read request body: {e}")))?
        .to_bytes();
    let (model, stream) = request_meta(&body_bytes);

    let row = match resolve_target(&st, headers, model.as_deref()).await {
        Ok(row) => row,
        Err(e) => {
            logs::record_failure(&st, &client, None, &method_str, path, 0, &e);
            return Err(e);
        }
    };
    let api_key = match st.decrypt_secret(&row.api_key_enc).await {
        Ok(api_key) => api_key,
        Err(error) => {
            logs::record_failure(
                &st,
                &client,
                Some(&row),
                &method_str,
                path,
                started.elapsed().as_millis() as i64,
                &error,
            );
            return Err(error);
        }
    };

    let base = crate::models::OPENCODE_BASE_URL;
    let path = path.trim_matches('/');
    let url = if path.is_empty() {
        base.to_string()
    } else {
        format!("{base}/{path}")
    };

    // Forward only headers that make sense upstream. Never forward the client's
    // own Authorization — the upstream gets ours instead.
    let mut fwd = HeaderMap::new();
    for h in [
        "content-type",
        "accept",
        "openai-organization",
        "openai-project",
        "anthropic-version",
        "anthropic-beta",
    ] {
        if let Some(v) = headers.get(h) {
            fwd.insert(h, v.clone());
        }
    }
    let auth = format!("Bearer {}", api_key.as_str())
        .parse::<axum::http::HeaderValue>()
        .map_err(|_| ApiError::Internal("failed to build Authorization header".into()))?;
    fwd.insert(axum::http::header::AUTHORIZATION, auth);

    let upstream_client = match st.client_for_key(&row).await {
        Ok(upstream_client) => upstream_client,
        Err(error) => {
            logs::record_failure(
                &st,
                &client,
                Some(&row),
                &method_str,
                path,
                started.elapsed().as_millis() as i64,
                &error,
            );
            return Err(error);
        }
    };
    let mut req = upstream_client.request(method, &url).headers(fwd);
    if !body_bytes.is_empty() {
        req = req.body(body_bytes);
    }
    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let err = ApiError::Upstream(format!("upstream unreachable: {e}"));
            logs::record_failure(
                &st,
                &client,
                Some(&row),
                &method_str,
                path,
                started.elapsed().as_millis() as i64,
                &err,
            );
            return Err(err);
        }
    };

    let status = resp.status();
    let latency_ms = started.elapsed().as_millis() as i64;
    let mut resp_headers = resp.headers().clone();
    // Strip hop-by-hop headers: we re-chunk the stream ourselves.
    for h in [
        "content-length",
        "transfer-encoding",
        "connection",
        "accept-encoding",
    ] {
        resp_headers.remove(h);
    }

    // An upstream authentication failure is a regular JSON error even when the
    // client requested SSE. Buffer all non-success responses so their message
    // is captured in the request log instead of disappearing into the stream.
    if should_forward_stream(stream, status) {
        // Streaming: forward each SSE frame as it arrives. Record the call with
        // TTFB latency now; token usage is backfilled when the stream ends.
        let log_id = logs::insert_log(
            &st,
            &LogInput {
                client: &client,
                route: Some(&row),
                method: &method_str,
                path,
                model: model.as_deref(),
                stream: true,
                status: status.as_u16(),
                latency_ms,
                prompt_tokens: None,
                completion_tokens: None,
                cached_tokens: None,
                cache_creation_tokens: None,
                error: None,
            },
        )
        .unwrap_or_default();
        let stream = resp.bytes_stream();
        let resp_body = Body::from_stream(logs::capture_usage_stream(stream, st, log_id, started));
        return Ok((status, resp_headers, resp_body).into_response());
    }

    // Non-stream: buffer the full response so we can read token usage / error.
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            let err = ApiError::Upstream(format!("upstream response read failed: {e}"));
            logs::record_failure(
                &st,
                &client,
                Some(&row),
                &method_str,
                path,
                latency_ms,
                &err,
            );
            return Err(err);
        }
    };
    let usage = logs::usage_from_bytes(&bytes);
    let error = if status.is_client_error() || status.is_server_error() {
        logs::error_from_bytes(&bytes)
    } else {
        None
    };
    let _ = logs::insert_log(
        &st,
        &LogInput {
            client: &client,
            route: Some(&row),
            method: &method_str,
            path,
            model: model.as_deref(),
            stream: false,
            status: status.as_u16(),
            latency_ms,
            prompt_tokens: usage.and_then(|u| u.prompt),
            completion_tokens: usage.and_then(|u| u.completion),
            cached_tokens: usage.and_then(|u| u.cached),
            cache_creation_tokens: usage.and_then(|u| u.cache_creation),
            error,
        },
    );
    Ok((status, resp_headers, Body::from(bytes)).into_response())
}

fn should_forward_stream(requested: bool, status: StatusCode) -> bool {
    requested && status.is_success()
}

/// Pull `model` / `stream` out of the request body for the log. Non-JSON
/// bodies just get `(None, false)`.
fn request_meta(body: &[u8]) -> (Option<String>, bool) {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return (None, false);
    };
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (model, stream)
}

/// Models exposed by OpenCode Zen do not all use the same wire protocol.
/// Keep this list in one place so special routes (notably `/messages`) can
/// decide whether to proxy natively or apply a compatibility conversion.
pub(crate) fn native_endpoint_for_model(model: &str) -> Option<&'static str> {
    match model {
        "gpt-5.6-luna" => Some("responses"),
        "minimax-m3" | "minimax-m2.7" | "minimax-m2.5" | "qwen3.8-max" | "qwen3.7-max"
        | "qwen3.7-plus" | "qwen3.6-plus" => Some("messages"),
        "grok-4.5" | "glm-5.2" | "glm-5.1" | "kimi-k3" | "kimi-k2.7-code" | "kimi-k2.6"
        | "deepseek-v4-pro" | "deepseek-v4-flash" | "mimo-v2.5" | "mimo-v2.5-pro" | "hy3" => {
            Some("chat/completions")
        }
        _ => None,
    }
}

/// Verify one raw client credential. A Bearer prefix is accepted
/// case-insensitively for callers that pass the complete header value.
pub fn authenticate_with(st: &AppState, raw: &str) -> Result<crate::db::ClientKeyRow, ApiError> {
    let raw = raw.trim();
    let key = raw
        .split_once(' ')
        .filter(|(scheme, key)| scheme.eq_ignore_ascii_case("bearer") && !key.is_empty())
        .map_or(raw, |(_, key)| key.trim());
    let hash = crate::crypto::hash_client_key(key);
    let row = st
        .db
        .client_key_by_hash(&hash)?
        .ok_or_else(|| ApiError::Unauthorized("invalid API key".into()))?;
    let _ = st.db.touch_client_key(&hash, crate::models::now_secs());
    Ok(row)
}

pub(crate) fn authenticate_client(
    st: &AppState,
    headers: &HeaderMap,
) -> Result<crate::db::ClientKeyRow, ApiError> {
    let credentials = credential_candidates(headers);
    if credentials.is_empty() {
        return Err(ApiError::Unauthorized("missing API key".into()));
    }
    for credential in credentials {
        match authenticate_with(st, credential) {
            Ok(client) => return Ok(client),
            Err(ApiError::Unauthorized(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(ApiError::Unauthorized("invalid API key".into()))
}

/// Accept both common SDK conventions. Keep both candidates so a stale header
/// injected by one client layer cannot shadow a valid credential from another.
fn credential_candidates(headers: &HeaderMap) -> Vec<&str> {
    let mut credentials = Vec::with_capacity(2);
    if let Some(value) = header_str(headers, "x-api-key").filter(|value| !value.trim().is_empty()) {
        credentials.push(value.trim());
    }
    if let Some(value) = header_str(headers, "authorization")
        && let Some((scheme, key)) = value.split_once(' ')
        && scheme.eq_ignore_ascii_case("bearer")
        && !key.trim().is_empty()
    {
        credentials.push(key.trim());
    }
    credentials
}

pub(crate) async fn resolve_target(
    st: &AppState,
    headers: &HeaderMap,
    model: Option<&str>,
) -> Result<crate::db::KeyRow, ApiError> {
    if let Some(id) = header_str(headers, "x-key-id") {
        if let Some(row) = st.db.get_key(id)? {
            return Ok(row);
        }
        return Err(ApiError::BadRequest(format!("x-key-id not found: {id}")));
    }
    if let Some(name) = header_str(headers, "x-key-name") {
        if let Some(row) = st.db.get_key_by_name(name)? {
            return Ok(row);
        }
        return Err(ApiError::BadRequest(format!(
            "x-key-name not found: {name}"
        )));
    }
    let rows = st.db.all_key_rows()?;
    let candidates = candidates_for_model(rows, model);
    if candidates.is_empty() {
        return Err(ApiError::BadRequest("no account configured".into()));
    }

    let cursor = st.account_cursor.fetch_add(1, Ordering::Relaxed);
    Ok(candidates[(cursor % candidates.len() as u64) as usize].clone())
}

/// Prefer accounts whose refreshed model cache advertises the requested model.
/// If none do (including empty/stale caches), keep the gateway usable by
/// balancing across the full pool and let the upstream validate the model.
fn candidates_for_model(
    rows: Vec<crate::db::KeyRow>,
    model: Option<&str>,
) -> Vec<crate::db::KeyRow> {
    let Some(model) = model.filter(|model| !model.is_empty()) else {
        return rows;
    };
    let matches: Vec<_> = rows
        .iter()
        .filter(|row| row.model_cache.iter().any(|item| item.id == model))
        .cloned()
        .collect();
    if matches.is_empty() { rows } else { matches }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::{
        candidates_for_model, credential_candidates, native_endpoint_for_model,
        should_forward_stream,
    };
    use crate::db::KeyRow;
    use crate::models::ModelInfo;
    use axum::http::{HeaderMap, StatusCode};

    fn key(name: &str, models: &[&str]) -> KeyRow {
        KeyRow {
            id: name.into(),
            name: name.into(),
            _base_url: String::new(),
            api_key_enc: String::new(),
            tags: vec![],
            notes: String::new(),
            model_cache: models
                .iter()
                .map(|id| ModelInfo {
                    id: (*id).into(),
                    owned_by: String::new(),
                })
                .collect(),
            is_default: false,
            created_at: 0,
            updated_at: 0,
            proxy_id: None,
            proxy_name: None,
        }
    }

    #[test]
    fn opencode_zen_models_select_their_native_endpoint() {
        assert_eq!(native_endpoint_for_model("gpt-5.6-luna"), Some("responses"));
        assert_eq!(native_endpoint_for_model("qwen3.8-max"), Some("messages"));
        assert_eq!(
            native_endpoint_for_model("deepseek-v4-pro"),
            Some("chat/completions")
        );
        assert_eq!(native_endpoint_for_model("unknown-model"), None);
    }

    #[test]
    fn load_balancing_pool_contains_every_account_supporting_the_model() {
        let rows = vec![
            key("a", &["shared"]),
            key("b", &["other"]),
            key("c", &["shared"]),
        ];

        let candidates = candidates_for_model(rows, Some("shared"));

        assert_eq!(
            candidates
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn load_balancing_pool_falls_back_to_all_accounts_for_unknown_model() {
        let rows = vec![key("a", &[]), key("b", &["known"])];

        let candidates = candidates_for_model(rows, Some("unknown"));

        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn streaming_authentication_errors_are_buffered_for_logging() {
        assert!(!should_forward_stream(true, StatusCode::UNAUTHORIZED));
        assert!(!should_forward_stream(true, StatusCode::FORBIDDEN));
        assert!(should_forward_stream(true, StatusCode::OK));
    }

    #[test]
    fn both_api_key_header_conventions_are_authentication_candidates() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "stale-key".parse().unwrap());
        headers.insert("authorization", "bEaReR valid-key".parse().unwrap());

        assert_eq!(
            credential_candidates(&headers),
            vec!["stale-key", "valid-key"]
        );
    }
}

/// OpenAI-style error body so client SDKs parse it.
pub(crate) fn openai_error(e: ApiError) -> Response {
    let status = e.status();
    let msg = e.message();
    let err_type = match status {
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => "invalid_request_error",
        StatusCode::UNAUTHORIZED => "authentication_error",
        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT => "api_connection_error",
        _ => "api_error",
    };
    (
        status,
        axum::Json(json!({
            "error": { "message": msg, "type": err_type, "code": null }
        })),
    )
        .into_response()
}
