use std::time::Instant;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::db::ClientKeyRow;
use crate::error::ApiError;
use crate::middleware::Unlocked;
use crate::routes::logs::{self, LogInput};
use crate::state::AppState;

/// Anthropic Messages compatibility endpoint backed by an OpenAI-compatible
/// `/chat/completions` upstream.
pub async fn messages(
    State(st): State<AppState>,
    _: Unlocked,
    headers: HeaderMap,
    Json(input): Json<Value>,
) -> Response {
    match messages_inner(st, &headers, input).await {
        Ok(response) => response,
        Err(error) => anthropic_error(error),
    }
}

async fn messages_inner(
    st: AppState,
    headers: &HeaderMap,
    input: Value,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    let client_key = authenticate(&st, headers)?;
    let model = input
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Zen's MiniMax and Qwen models speak Anthropic Messages natively. Sending
    // those through the legacy Messages -> Chat Completions adapter changes the
    // payload and targets the wrong upstream endpoint, so preserve the request
    // and response byte-for-byte for these models.
    if super::proxy::native_endpoint_for_model(&model) == Some("messages") {
        let body = serde_json::to_vec(&input)
            .map_err(|e| ApiError::Internal(format!("serialize request body: {e}")))?;
        return super::proxy::proxy_inner(
            st,
            Method::POST,
            "messages",
            headers,
            Body::from(body),
            Some(client_key),
        )
        .await;
    }
    let row = match super::proxy::resolve_target(&st, headers, Some(&model)).await {
        Ok(row) => row,
        Err(e) => {
            logs::record_failure(&st, &client_key, None, "POST", "/messages", 0, &e);
            return Err(e);
        }
    };
    let upstream_key = st.decrypt_secret(&row.api_key_enc).await?;
    let stream = input
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let request = to_openai_request(&input)?;
    let url = format!("{}/chat/completions", crate::models::OPENCODE_BASE_URL);
    let upstream_client = st.client_for_key(&row).await?;
    let upstream = match upstream_client
        .post(url)
        .bearer_auth(upstream_key.as_str())
        .json(&request)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            let err = ApiError::Upstream(format!("upstream unreachable: {e}"));
            logs::record_failure(
                &st,
                &client_key,
                Some(&row),
                "POST",
                "/messages",
                started.elapsed().as_millis() as i64,
                &err,
            );
            return Err(err);
        }
    };

    let status = upstream.status();
    let latency_ms = started.elapsed().as_millis() as i64;

    if !status.is_success() {
        let body = upstream.text().await.unwrap_or_default();
        let error = logs::error_from_bytes(body.as_bytes()).unwrap_or_else(|| status.to_string());
        let _ = logs::insert_log(
            &st,
            &LogInput {
                client: &client_key,
                route: Some(&row),
                method: "POST",
                path: "/messages",
                model: Some(&model),
                stream,
                status: status.as_u16(),
                latency_ms,
                prompt_tokens: None,
                completion_tokens: None,
                error: Some(error),
            },
        );
        return Ok((status, body).into_response());
    }
    if stream {
        let log_id = logs::insert_log(
            &st,
            &LogInput {
                client: &client_key,
                route: Some(&row),
                method: "POST",
                path: "/messages",
                model: Some(&model),
                stream: true,
                status: 200,
                latency_ms,
                prompt_tokens: None,
                completion_tokens: None,
                error: None,
            },
        )
        .unwrap_or_default();
        return Ok(stream_response(upstream, model, st, log_id, started));
    }
    let value: Value = upstream
        .json()
        .await
        .map_err(|e| ApiError::Upstream(format!("invalid upstream response: {e}")))?;
    let usage = logs::usage_from_json(&value);
    let _ = logs::insert_log(
        &st,
        &LogInput {
            client: &client_key,
            route: Some(&row),
            method: "POST",
            path: "/messages",
            model: Some(&model),
            stream: false,
            status: 200,
            latency_ms,
            prompt_tokens: usage.and_then(|u| u.0),
            completion_tokens: usage.and_then(|u| u.1),
            error: None,
        },
    );
    Ok(Json(to_anthropic_response(value)?).into_response())
}

fn authenticate(st: &AppState, headers: &HeaderMap) -> Result<ClientKeyRow, ApiError> {
    let raw = headers
        .get("x-api-key")
        .or_else(|| headers.get(axum::http::header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized("missing API key".into()))?;
    super::proxy::authenticate_with(st, raw)
}

fn to_openai_request(input: &Value) -> Result<Value, ApiError> {
    let mut messages = Vec::new();
    if let Some(system) = input.get("system") {
        messages.push(json!({"role":"system", "content": content_text(system)}));
    }
    for message in input
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::BadRequest("messages is required".into()))?
    {
        messages.push(json!({
            "role": message.get("role").and_then(Value::as_str).unwrap_or("user"),
            "content": content_text(message.get("content").unwrap_or(&Value::Null))
        }));
    }
    let mut out = json!({
        "model": input.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages,
        "stream": input.get("stream").cloned().unwrap_or(json!(false)),
        "max_tokens": input.get("max_tokens").cloned().unwrap_or(json!(1024))
    });
    for key in ["temperature", "top_p"] {
        if let Some(value) = input.get(key) {
            out[key] = value.clone();
        }
    }
    if let Some(stops) = input.get("stop_sequences") {
        out["stop"] = stops.clone();
    }
    if let Some(tools) = input.get("tools").and_then(Value::as_array) {
        out["tools"] = Value::Array(tools.iter().map(|tool| json!({"type":"function", "function": {
            "name": tool.get("name"), "description": tool.get("description"), "parameters": tool.get("input_schema")
        }})).collect());
    }
    Ok(out)
}

fn content_text(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    value
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                    Some("text") => block
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    Some("tool_result") => {
                        Some(content_text(block.get("content").unwrap_or(&Value::Null)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn to_anthropic_response(value: Value) -> Result<Value, ApiError> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|v| v.first())
        .ok_or_else(|| ApiError::Upstream("upstream returned no choices".into()))?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    let mut content = vec![
        json!({"type":"text", "text": message.get("content").and_then(Value::as_str).unwrap_or("")}),
    ];
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let function = &call["function"];
            let args = function
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}));
            content.push(json!({"type":"tool_use", "id":call.get("id"), "name":function.get("name"), "input":args}));
        }
    }
    let finish = choice.get("finish_reason").and_then(Value::as_str);
    Ok(json!({
        "id": value.get("id"), "type":"message", "role":"assistant", "model":value.get("model"),
        "content":content, "stop_reason": match finish { Some("length") => "max_tokens", Some("tool_calls") => "tool_use", _ => "end_turn" },
        "stop_sequence":null,
        "usage":{"input_tokens":value.pointer("/usage/prompt_tokens").cloned().unwrap_or(json!(0)), "output_tokens":value.pointer("/usage/completion_tokens").cloned().unwrap_or(json!(0))}
    }))
}

fn stream_response(
    upstream: reqwest::Response,
    model: String,
    st: AppState,
    log_id: String,
    started: Instant,
) -> Response {
    let mut source = upstream.bytes_stream();
    let output = async_stream::stream! {
        let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
        yield Ok::<Bytes, std::io::Error>(event("message_start", json!({"type":"message_start","message":{"id":id,"type":"message","role":"assistant","model":model,"content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":0,"output_tokens":0}}})));
        yield Ok(event("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})));
        let mut buffer = String::new();
        let mut stop_reason = "end_turn";
        let mut usage: Option<(Option<i64>, Option<i64>)> = None;
        let mut first_token_ms: Option<i64> = None;
        while let Some(chunk) = source.next().await {
            let Ok(chunk) = chunk else { break };
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..=pos);
                let Some(data) = line.strip_prefix("data: ") else { continue };
                if data == "[DONE]" { continue; }
                let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
                if let Some(u) = logs::usage_from_json(&value) { usage = Some(u); }
                if let Some(text) = value.pointer("/choices/0/delta/content").and_then(Value::as_str) {
                    if first_token_ms.is_none() && !text.is_empty() {
                        first_token_ms = Some(started.elapsed().as_millis() as i64);
                    }
                    yield Ok(event("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}})));
                }
                if let Some(reason) = value.pointer("/choices/0/finish_reason").and_then(Value::as_str) {
                    stop_reason = if reason == "length" { "max_tokens" } else { "end_turn" };
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
        yield Ok(event("content_block_stop", json!({"type":"content_block_stop","index":0})));
        yield Ok(event("message_delta", json!({"type":"message_delta","delta":{"stop_reason":stop_reason,"stop_sequence":null},"usage":{"output_tokens":0}})));
        yield Ok(event("message_stop", json!({"type":"message_stop"})));
    };
    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
        ],
        Body::from_stream(output),
    )
        .into_response()
}

fn event(name: &str, value: Value) -> Bytes {
    Bytes::from(format!("event: {name}\ndata: {value}\n\n"))
}

fn anthropic_error(error: ApiError) -> Response {
    let status = error.status();
    let kind = if status == StatusCode::UNAUTHORIZED {
        "authentication_error"
    } else if status == StatusCode::BAD_REQUEST {
        "invalid_request_error"
    } else {
        "api_error"
    };
    (
        status,
        Json(json!({"type":"error", "error":{"type":kind,"message":error.message()}})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_anthropic_request_to_openai() {
        let input = json!({
            "model":"gpt-4o", "max_tokens":128, "system":"Be concise",
            "messages":[{"role":"user","content":[{"type":"text","text":"Hello"}]}]
        });
        let output = to_openai_request(&input).unwrap();
        assert_eq!(
            output["messages"][0],
            json!({"role":"system","content":"Be concise"})
        );
        assert_eq!(
            output["messages"][1],
            json!({"role":"user","content":"Hello"})
        );
        assert_eq!(output["max_tokens"], 128);
    }

    #[test]
    fn converts_openai_response_to_anthropic() {
        let input = json!({
            "id":"chatcmpl-1", "model":"gpt-4o",
            "choices":[{"message":{"role":"assistant","content":"Hi"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":3,"completion_tokens":2}
        });
        let output = to_anthropic_response(input).unwrap();
        assert_eq!(output["type"], "message");
        assert_eq!(output["content"][0]["text"], "Hi");
        assert_eq!(output["usage"]["output_tokens"], 2);
        assert_eq!(output["stop_reason"], "end_turn");
    }
}
