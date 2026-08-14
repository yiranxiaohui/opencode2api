use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use futures_util::Stream;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::time::Instant;
use uuid::Uuid;

use crate::db::{ClientKeyRow, KeyRow, LogFilter, RequestLogRow};
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{LogListResponse, LogStatsResponse, OkResponse, RequestLogRecord, now_secs};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub client: Option<String>,
    pub key: Option<String>,
    pub model: Option<String>,
    pub status: Option<i64>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Stats ignore limit/offset and aggregate over all matching rows.
fn filter_from(q: LogQuery) -> LogFilter {
    LogFilter {
        client_key_id: q.client,
        route_key_id: q.key,
        model: q
            .model
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        status: q.status,
        limit: q.limit.unwrap_or(200).clamp(1, 500),
        offset: q.offset.unwrap_or(0).max(0),
    }
}

pub async fn list(
    State(st): State<AppState>,
    _: ManagementAuth,
    Query(q): Query<LogQuery>,
) -> Result<Json<LogListResponse>, ApiError> {
    let filter = filter_from(q);
    let (rows, total) = st.db.list_request_logs(&filter)?;
    Ok(Json(LogListResponse {
        items: rows.iter().map(RequestLogRecord::from_row).collect(),
        total,
    }))
}

pub async fn stats(
    State(st): State<AppState>,
    _: ManagementAuth,
    Query(q): Query<LogQuery>,
) -> Result<Json<LogStatsResponse>, ApiError> {
    let filter = filter_from(q);
    let (totals, by_model, by_client) = st.db.log_stats(&filter)?;
    Ok(Json(LogStatsResponse {
        totals,
        by_model,
        by_client,
    }))
}

pub async fn clear(
    State(st): State<AppState>,
    _: ManagementAuth,
) -> Result<Json<OkResponse>, ApiError> {
    st.db.clear_request_logs()?;
    Ok(Json(OkResponse { ok: true }))
}

// ---------------------------------------------------------------------------
// Shared helpers used by the proxy / messages handlers to record one call.
// Logging is best-effort: a failed insert must never break the proxied reply.
// ---------------------------------------------------------------------------

pub struct LogInput<'a> {
    pub client: &'a ClientKeyRow,
    pub route: Option<&'a KeyRow>,
    pub method: &'a str,
    pub path: &'a str,
    pub model: Option<&'a str>,
    pub stream: bool,
    pub status: u16,
    pub latency_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub error: Option<String>,
}

pub fn insert_log(st: &AppState, input: &LogInput<'_>) -> Result<String, ApiError> {
    let id = Uuid::new_v4().to_string();
    st.db.insert_request_log(&RequestLogRow {
        id: id.clone(),
        created_at: now_secs(),
        client_key_id: Some(input.client.id.clone()),
        client_key_name: input.client.name.clone(),
        route_key_id: input.route.map(|r| r.id.clone()),
        route_key_name: input.route.map(|r| r.name.clone()),
        method: input.method.to_string(),
        path: input.path.to_string(),
        model: input.model.map(str::to_string),
        stream: input.stream,
        status: input.status as i64,
        latency_ms: input.latency_ms,
        first_token_ms: None,
        prompt_tokens: input.prompt_tokens,
        completion_tokens: input.completion_tokens,
        cached_tokens: input.cached_tokens,
        cache_creation_tokens: input.cache_creation_tokens,
        error: input.error.clone(),
    })?;
    Ok(id)
}

/// Record a request that failed before reaching upstream (target missing,
/// upstream unreachable, response read error).
pub fn record_failure(
    st: &AppState,
    client: &ClientKeyRow,
    route: Option<&KeyRow>,
    method: &str,
    path: &str,
    latency_ms: i64,
    error: &ApiError,
) {
    let _ = insert_log(
        st,
        &LogInput {
            client,
            route,
            method,
            path,
            model: None,
            stream: false,
            status: error.status().as_u16(),
            latency_ms,
            prompt_tokens: None,
            completion_tokens: None,
            cached_tokens: None,
            cache_creation_tokens: None,
            error: Some(error.message()),
        },
    );
}

/// Record a rejected gateway request without retaining the presented secret.
/// There is no client row to reference because authentication did not succeed.
pub fn record_auth_failure(st: &AppState, method: &str, path: &str, error: &ApiError) {
    let _ = st.db.insert_request_log(&RequestLogRow {
        id: Uuid::new_v4().to_string(),
        created_at: now_secs(),
        client_key_id: None,
        client_key_name: "未认证客户端".into(),
        route_key_id: None,
        route_key_name: None,
        method: method.into(),
        path: path.into(),
        model: None,
        stream: false,
        status: error.status().as_u16() as i64,
        latency_ms: 0,
        first_token_ms: None,
        prompt_tokens: None,
        completion_tokens: None,
        cached_tokens: None,
        cache_creation_tokens: None,
        error: Some(error.message()),
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt: Option<i64>,
    pub completion: Option<i64>,
    pub cached: Option<i64>,
    pub cache_creation: Option<i64>,
}

impl Usage {
    fn merge(self, newer: Self) -> Self {
        Self {
            prompt: newer.prompt.or(self.prompt),
            completion: newer.completion.or(self.completion),
            cached: newer.cached.or(self.cached),
            cache_creation: newer.cache_creation.or(self.cache_creation),
        }
    }
}

pub fn usage_from_json(value: &Value) -> Option<Usage> {
    let cache_read = value
        .pointer("/usage/cache_read_input_tokens")
        .and_then(Value::as_i64);
    let cache_creation = value
        .pointer("/usage/cache_creation_input_tokens")
        .and_then(Value::as_i64);
    let input = value.pointer("/usage/input_tokens").and_then(Value::as_i64);
    // Anthropic reports uncached input, cache reads and cache writes separately.
    // Normalize prompt to the total input footprint so hit-rate calculations
    // have the same denominator as OpenAI's prompt/input token total.
    let anthropic_prompt =
        input.map(|tokens| tokens + cache_read.unwrap_or(0) + cache_creation.unwrap_or(0));
    let usage = Usage {
        prompt: value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_i64)
            .or(anthropic_prompt),
        completion: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_i64)
            .or_else(|| {
                value
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_i64)
            }),
        cached: value
            .pointer("/usage/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
            .or_else(|| {
                value
                    .pointer("/usage/input_tokens_details/cached_tokens")
                    .and_then(Value::as_i64)
            })
            .or_else(|| {
                value
                    .pointer("/usage/prompt_cache_hit_tokens")
                    .and_then(Value::as_i64)
            })
            .or(cache_read),
        cache_creation,
    };
    if usage != Usage::default() {
        Some(usage)
    } else {
        None
    }
}

pub fn usage_from_bytes(bytes: &[u8]) -> Option<Usage> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    usage_from_json(&value)
}

/// Best-effort extraction of the upstream `error.message` from a non-stream
/// error body. Truncated so a hostile upstream cannot bloat the log row.
pub fn error_from_bytes(bytes: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(|s| s.chars().take(500).collect())
}

/// Wrap a raw upstream SSE stream, forwarding every chunk untouched while
/// scanning for the final `usage` object. When the stream finishes, backfill
/// the log row's token counts.
pub fn capture_usage_stream<S>(
    stream: S,
    st: AppState,
    log_id: String,
    started: Instant,
) -> impl Stream<Item = Result<Bytes, reqwest::Error>>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let mut stream = Box::pin(stream);
    let mut buffer = String::new();
    let mut finalizer = StreamLogFinalizer {
        st,
        log_id,
        started,
        usage: None,
        first_token_ms: None,
    };
    async_stream::stream! {
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].trim().to_string();
                        buffer.drain(..=pos);
                        if let Some(data) = line.strip_prefix("data:").map(str::trim_start)
                            && data != "[DONE]"
                            && let Ok(value) = serde_json::from_str::<Value>(data)
                        {
                            if finalizer.first_token_ms.is_none() && has_output_delta(&value) {
                                finalizer.first_token_ms =
                                    Some(started.elapsed().as_millis() as i64);
                            }
                            if let Some(u) = usage_from_json(&value) {
                                finalizer.usage = Some(
                                    finalizer
                                        .usage
                                        .map_or(u, |current| current.merge(u)),
                                );
                            }
                        }
                    }
                    yield Ok(bytes);
                }
                Err(e) => {
                    yield Err(e);
                    break;
                }
            }
        }
    }
}

struct StreamLogFinalizer {
    st: AppState,
    log_id: String,
    started: Instant,
    usage: Option<Usage>,
    first_token_ms: Option<i64>,
}

impl Drop for StreamLogFinalizer {
    fn drop(&mut self) {
        let usage = self.usage.unwrap_or_default();
        let _ = self.st.db.finalize_stream_log(
            &self.log_id,
            self.first_token_ms,
            self.started.elapsed().as_millis() as i64,
            usage.prompt,
            usage.completion,
            usage.cached,
            usage.cache_creation,
        );
    }
}

fn has_output_delta(value: &Value) -> bool {
    value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
        || value
            .pointer("/choices/0/delta/reasoning_content")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        || value
            .pointer("/choices/0/delta/reasoning")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty())
        || value.pointer("/choices/0/delta/tool_calls").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_cache_usage_from_all_supported_protocols() {
        let chat = json!({"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":80}}});
        assert_eq!(usage_from_json(&chat).unwrap().cached, Some(80));

        let responses = json!({"usage":{"input_tokens":100,"output_tokens":20,"input_tokens_details":{"cached_tokens":70}}});
        assert_eq!(usage_from_json(&responses).unwrap().cached, Some(70));

        let messages = json!({"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":60,"cache_creation_input_tokens":30}});
        let usage = usage_from_json(&messages).unwrap();
        assert_eq!(usage.cached, Some(60));
        assert_eq!(usage.cache_creation, Some(30));

        let deepseek = json!({"usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_cache_hit_tokens":75,"prompt_cache_miss_tokens":25}});
        assert_eq!(usage_from_json(&deepseek).unwrap().cached, Some(75));
    }

    #[test]
    fn merges_usage_split_across_stream_events() {
        let input = Usage {
            prompt: Some(100),
            cached: Some(75),
            ..Usage::default()
        };
        let output = Usage {
            completion: Some(20),
            ..Usage::default()
        };
        assert_eq!(
            input.merge(output),
            Usage {
                prompt: Some(100),
                completion: Some(20),
                cached: Some(75),
                cache_creation: None,
            }
        );
    }

    #[test]
    fn reasoning_delta_counts_as_first_output() {
        let delta = json!({"choices":[{"delta":{"reasoning_content":"thinking"}}]});
        assert!(has_output_delta(&delta));
    }
}
