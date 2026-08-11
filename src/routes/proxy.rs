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

/// Unified OpenAI-compatible gateway: forwards `/v1/*` to the target key.
/// Target chosen by `X-Key-Id` / `X-Key-Name` headers, else the default key.
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
        None => authenticate_client(&st, headers)?,
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
    let api_key = st.decrypt_secret(&row.api_key_enc).await?;

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

    let upstream_client = st.client_for_key(&row).await?;
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

    if stream {
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
            prompt_tokens: usage.and_then(|u| u.0),
            completion_tokens: usage.and_then(|u| u.1),
            error,
        },
    );
    Ok((status, resp_headers, Body::from(bytes)).into_response())
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

/// Shared bearer-token verification for both proxy endpoints.
pub fn authenticate_with(st: &AppState, raw: &str) -> Result<crate::db::ClientKeyRow, ApiError> {
    let key = raw.strip_prefix("Bearer ").unwrap_or(raw);
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
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("missing Authorization bearer token".into()))?;
    let (scheme, api_key) = value
        .split_once(' ')
        .ok_or_else(|| ApiError::Unauthorized("invalid Authorization bearer token".into()))?;
    if !scheme.eq_ignore_ascii_case("bearer") || api_key.is_empty() {
        return Err(ApiError::Unauthorized(
            "invalid Authorization bearer token".into(),
        ));
    }
    authenticate_with(st, api_key)
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
    if let Some(row) = st.db.get_default_key()? {
        return Ok(row);
    }

    let rows = st.db.all_key_rows()?;
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        let mut matches = rows
            .iter()
            .filter(|row| row.model_cache.iter().any(|item| item.id == model));
        if let Some(row) = matches.next() {
            if matches.next().is_none() {
                return Ok(row.clone());
            }
        }
    }
    if rows.len() == 1 {
        return Ok(rows.into_iter().next().expect("one route"));
    }
    Err(ApiError::BadRequest(
        "no target account: set a default account, use a model unique to one account, or pass X-Key-Id / X-Key-Name headers".into(),
    ))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::native_endpoint_for_model;

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
