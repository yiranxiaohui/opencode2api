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
use crate::middleware::Unlocked;
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
    _: Unlocked,
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
    _: Unlocked,
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

pub async fn clear(State(st): State<AppState>, _: Unlocked) -> Result<Json<OkResponse>, ApiError> {
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
            error: Some(error.message()),
        },
    );
}

pub fn usage_from_json(value: &Value) -> Option<(Option<i64>, Option<i64>)> {
    let prompt = value
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_i64);
    let completion = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_i64);
    if prompt.is_some() || completion.is_some() {
        Some((prompt, completion))
    } else {
        None
    }
}

pub fn usage_from_bytes(bytes: &[u8]) -> Option<(Option<i64>, Option<i64>)> {
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
    let mut usage: Option<(Option<i64>, Option<i64>)> = None;
    let mut first_token_ms: Option<i64> = None;
    async_stream::stream! {
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].trim().to_string();
                        buffer.drain(..=pos);
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data != "[DONE]" {
                                if let Ok(value) = serde_json::from_str::<Value>(data) {
                                    if first_token_ms.is_none() && has_output_delta(&value) {
                                        first_token_ms = Some(started.elapsed().as_millis() as i64);
                                    }
                                    if let Some(u) = usage_from_json(&value) {
                                        usage = Some(u);
                                    }
                                }
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
        let (prompt, completion) = usage.unwrap_or((None, None));
        let _ = st.db.finalize_stream_log(
            &log_id,
            first_token_ms,
            started.elapsed().as_millis() as i64,
            prompt,
            completion,
        );
    }
}

fn has_output_delta(value: &Value) -> bool {
    value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
        || value.pointer("/choices/0/delta/tool_calls").is_some()
}
