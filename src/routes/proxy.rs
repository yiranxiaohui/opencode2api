use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::middleware::Unlocked;
use crate::routes::logs::{self, LogInput};
use crate::state::AppState;

/// Unified OpenAI-compatible gateway: forwards `/v1/*` to a sticky-session key.
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
        key_enc: None,
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
    let body_bytes = include_stream_usage(body_bytes, path, stream);
    let affinity = affinity_key(headers, &client.id, model.as_deref());

    let row = match resolve_target(&st, headers, model.as_deref(), &affinity).await {
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
    insert_upstream_auth(&mut fwd, api_key.as_str(), path)?;

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

/// OpenCode's OpenAI-compatible endpoints use Bearer auth, while native
/// Anthropic Messages clients commonly use X-API-Key. Send both conventions
/// only for `/messages`; both values come from the selected upstream account,
/// never from the gateway client's credential.
fn insert_upstream_auth(
    headers: &mut HeaderMap,
    api_key: &str,
    path: &str,
) -> Result<(), ApiError> {
    let bearer = format!("Bearer {api_key}")
        .parse::<axum::http::HeaderValue>()
        .map_err(|_| ApiError::Internal("failed to build Authorization header".into()))?;
    headers.insert(axum::http::header::AUTHORIZATION, bearer);
    if path == "messages" {
        let api_key = api_key
            .parse::<axum::http::HeaderValue>()
            .map_err(|_| ApiError::Internal("failed to build X-API-Key header".into()))?;
        headers.insert("x-api-key", api_key);
    }
    Ok(())
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

/// OpenAI-compatible chat streams only include their final usage frame when
/// explicitly requested. Ask for it so request logs can record token usage,
/// while leaving non-chat, non-streaming and non-JSON bodies untouched.
fn include_stream_usage(body: Bytes, path: &str, stream: bool) -> Bytes {
    if !stream || path.trim_matches('/') != "chat/completions" {
        return body;
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    let Some(request) = value.as_object_mut() else {
        return body;
    };
    let options = request.entry("stream_options").or_insert_with(|| json!({}));
    if !options.is_object() {
        *options = json!({});
    }
    options
        .as_object_mut()
        .expect("stream_options was replaced with an object")
        .insert("include_usage".into(), Value::Bool(true));

    serde_json::to_vec(&value).map(Bytes::from).unwrap_or(body)
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
    affinity: &str,
) -> Result<crate::db::KeyRow, ApiError> {
    if let Some(id) = header_str(headers, "x-key-id") {
        if let Some(row) = st.db.get_key(id)? {
            if !row.is_enabled {
                return Err(ApiError::BadRequest(format!(
                    "account is disabled: {}",
                    row.name
                )));
            }
            return Ok(row);
        }
        return Err(ApiError::BadRequest(format!("x-key-id not found: {id}")));
    }
    if let Some(name) = header_str(headers, "x-key-name") {
        if let Some(row) = st.db.get_key_by_name(name)? {
            if !row.is_enabled {
                return Err(ApiError::BadRequest(format!(
                    "account is disabled: {}",
                    row.name
                )));
            }
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

    Ok(select_sticky_account(&candidates, affinity)
        .expect("non-empty candidates")
        .clone())
}

/// Build a stable routing key. Explicit session headers allow one client to
/// spread independent conversations across accounts while keeping every turn
/// of a conversation on the same account. Without one, client + model remains
/// sticky, favoring cache reuse over per-request distribution.
pub(crate) fn affinity_key(headers: &HeaderMap, client_id: &str, model: Option<&str>) -> String {
    let session = header_str(headers, "x-session-id")
        .or_else(|| header_str(headers, "x-conversation-id"))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    format!(
        "client:{}\0model:{}\0session:{}",
        client_id,
        model.unwrap_or_default(),
        session.unwrap_or_default()
    )
}

/// Rendezvous (highest-random-weight) hash of one account under an affinity key.
fn rendezvous_hash(affinity: &str, id: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(affinity.as_bytes());
    hasher.update([0]);
    hasher.update(id.as_bytes());
    let digest = hasher.finalize();
    u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix"))
}

/// Highest-random-weight hashing provides deterministic affinity independent of
/// database ordering and minimizes remapping when accounts are added to or
/// removed from the eligible model pool.
fn select_sticky_account<'a>(
    candidates: &'a [crate::db::KeyRow],
    affinity: &str,
) -> Option<&'a crate::db::KeyRow> {
    candidates
        .iter()
        .max_by_key(|row| rendezvous_hash(affinity, &row.id))
}

/// Accounts in `candidates` (already enabled + model-matched) that are not in
/// quota cooldown, ordered by descending rendezvous hash. First = the sticky
/// winner; the remaining order is the deterministic failover order for the
/// affinity key. Independent of input order.
// This helper is not yet reachable from production code; the automatic
// account-failover routing (Task 4) will call `ordered_candidates`, so silence
// the dead-code warning until then.
#[allow(dead_code)]
fn ordered_candidates<'a, F>(
    candidates: &'a [crate::db::KeyRow],
    affinity: &str,
    in_cooldown: F,
) -> Vec<&'a crate::db::KeyRow>
where
    F: Fn(&str) -> bool,
{
    let mut ordered: Vec<&crate::db::KeyRow> = candidates
        .iter()
        .filter(|row| !in_cooldown(&row.id))
        .collect();
    ordered.sort_by(|a, b| rendezvous_hash(affinity, &b.id).cmp(&rendezvous_hash(affinity, &a.id)));
    ordered
}

/// Prefer accounts whose refreshed model cache advertises the requested model.
/// If none do (including empty/stale caches), keep the gateway usable by
/// balancing across the full pool and let the upstream validate the model.
fn candidates_for_model(
    rows: Vec<crate::db::KeyRow>,
    model: Option<&str>,
) -> Vec<crate::db::KeyRow> {
    let rows: Vec<_> = rows.into_iter().filter(|row| row.is_enabled).collect();
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

/// Substrings that mark an upstream error body as "account quota exhausted".
/// Matched case-insensitively against OpenAI-style `error.message`/`type`/`code`.
pub(crate) const QUOTA_KEYWORDS: &[&str] = &[
    "quota",
    "insufficient",
    "balance",
    "payment",
    "billing",
    "credit",
    "exhausted",
    "额度",
    "余额",
];

/// How long an account that returned a quota-exhaustion error is skipped.
pub(crate) const QUOTA_COOLDOWN_SECS: i64 = 900;

/// Classify an upstream non-success response as quota exhaustion. HTTP 402 is a
/// hard signal; other 4xx bodies are scanned in the OpenAI error fields only.
/// Conservative by design: plain rate limiting (`rate_limit_exceeded`), missing
/// models, and 5xx must never trigger failover.
// This classifier is not yet reachable from production code; the automatic
// account-failover loop (Tasks 5/6) will call `is_quota_error` (which in turn
// reads `QUOTA_KEYWORDS`), so silence the dead-code warning until then.
#[allow(dead_code)]
pub(crate) fn is_quota_error(status: StatusCode, body: &[u8]) -> bool {
    if status == StatusCode::PAYMENT_REQUIRED {
        return true;
    }
    if !status.is_client_error() {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(err) = value.get("error") else {
        return false;
    };
    let haystack = ["message", "type", "code"]
        .into_iter()
        .filter_map(|key| err.get(key).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    QUOTA_KEYWORDS
        .iter()
        .any(|keyword| haystack.contains(keyword))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::{
        affinity_key, candidates_for_model, credential_candidates, include_stream_usage,
        insert_upstream_auth, is_quota_error, native_endpoint_for_model, ordered_candidates,
        select_sticky_account, should_forward_stream,
    };
    use crate::db::KeyRow;
    use crate::models::ModelInfo;
    use axum::body::Bytes;
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
            _is_default: false,
            is_enabled: true,
            created_at: 0,
            updated_at: 0,
            proxy_id: None,
            proxy_name: None,
            cookie_enc: None,
            workspace_id: None,
            usage_cache: None,
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
    fn disabled_accounts_are_excluded_from_routing() {
        let enabled = key("enabled", &["model-a"]);
        let mut disabled = key("disabled", &["model-a"]);
        disabled.is_enabled = false;

        let candidates = candidates_for_model(vec![disabled, enabled], Some("model-a"));

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "enabled");
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

    #[test]
    fn sticky_routing_is_stable_across_requests_and_candidate_order() {
        let candidates = vec![
            key("a", &["shared"]),
            key("b", &["shared"]),
            key("c", &["shared"]),
        ];
        let selected = select_sticky_account(&candidates, "client:model:session")
            .unwrap()
            .id
            .clone();
        let reversed = candidates.into_iter().rev().collect::<Vec<_>>();

        assert_eq!(
            select_sticky_account(&reversed, "client:model:session")
                .unwrap()
                .id,
            selected
        );
    }

    #[test]
    fn explicit_session_id_changes_the_affinity_key() {
        let mut first = HeaderMap::new();
        first.insert("x-session-id", "conversation-a".parse().unwrap());
        let mut second = HeaderMap::new();
        second.insert("x-conversation-id", "conversation-b".parse().unwrap());

        assert_ne!(
            affinity_key(&first, "client", Some("model")),
            affinity_key(&second, "client", Some("model"))
        );
        assert_eq!(
            affinity_key(&HeaderMap::new(), "client", Some("model")),
            affinity_key(&HeaderMap::new(), "client", Some("model"))
        );
    }

    #[test]
    fn native_messages_sends_both_upstream_auth_conventions() {
        let mut messages = HeaderMap::new();
        insert_upstream_auth(&mut messages, "upstream-secret", "messages").unwrap();
        assert_eq!(messages["authorization"], "Bearer upstream-secret");
        assert_eq!(messages["x-api-key"], "upstream-secret");

        let mut chat = HeaderMap::new();
        insert_upstream_auth(&mut chat, "upstream-secret", "chat/completions").unwrap();
        assert_eq!(chat["authorization"], "Bearer upstream-secret");
        assert!(!chat.contains_key("x-api-key"));
    }

    #[test]
    fn quota_errors_are_recognized_without_false_positives() {
        let quota_body =
            br#"{"error":{"message":"You have exhausted your monthly quota","type":"insufficient_quota","code":null}}"#;
        // HTTP 402 is a hard signal regardless of body.
        assert!(is_quota_error(StatusCode::PAYMENT_REQUIRED, b"{}"));
        // 429 with quota semantics in error fields.
        assert!(is_quota_error(StatusCode::TOO_MANY_REQUESTS, quota_body));
        // Chinese-language body.
        assert!(is_quota_error(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"message":"余额不足"}}"#.as_bytes()
        ));
        // Plain rate limiting must NOT trigger failover.
        assert!(!is_quota_error(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":{"message":"Rate limit exceeded","type":"rate_limit_exceeded"}}"#
        ));
        // Model-not-found / server errors / success must not.
        assert!(!is_quota_error(
            StatusCode::NOT_FOUND,
            br#"{"error":{"message":"model not found"}}"#
        ));
        assert!(!is_quota_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            quota_body
        ));
        assert!(!is_quota_error(StatusCode::OK, quota_body));
        assert!(!is_quota_error(StatusCode::TOO_MANY_REQUESTS, b"not json"));
    }

    #[test]
    fn ordered_candidates_are_deterministic_and_match_sticky_first() {
        let rows = vec![key("a", &["m"]), key("b", &["m"]), key("c", &["m"])];
        let affinity = "client:model:session";
        let no_cooldown = |_: &str| false;
        let order = ordered_candidates(&rows, affinity, no_cooldown);
        let ids: Vec<&str> = order.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids.len(), 3);
        // First choice is exactly the current sticky selector's choice.
        assert_eq!(
            select_sticky_account(&rows, affinity).unwrap().id.as_str(),
            ids[0]
        );
        // Order is independent of candidate input order.
        let reversed: Vec<KeyRow> = rows.iter().rev().cloned().collect();
        let reversed_ids: Vec<&str> = ordered_candidates(&reversed, affinity, no_cooldown)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(ids, reversed_ids);
    }

    #[test]
    fn ordered_candidates_skip_accounts_in_cooldown() {
        let rows = vec![key("a", &["m"]), key("b", &["m"]), key("c", &["m"])];
        let cool_b = |id: &str| id == "b";
        let ids: Vec<&str> = ordered_candidates(&rows, "k", cool_b)
            .into_iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids.contains(&"b"));
    }

    #[test]
    fn requests_usage_for_streaming_chat_completions() {
        let body =
            Bytes::from_static(br#"{"model":"deepseek-v4-flash","stream":true,"messages":[]}"#);
        let body = include_stream_usage(body, "/chat/completions/", true);
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value.pointer("/stream_options/include_usage"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn preserves_non_streaming_request_body() {
        let body = Bytes::from_static(br#"{"stream":false,"messages":[]}"#);
        assert_eq!(
            include_stream_usage(body.clone(), "chat/completions", false),
            body
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
