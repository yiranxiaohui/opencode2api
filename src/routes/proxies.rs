use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use std::time::Instant;
use tokio::net::TcpStream;
use tokio::time::{Duration, timeout};
use uuid::Uuid;

use crate::db::ProxyData;
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{
    OkResponse, ProxyInput, ProxyRecord, ProxyTestInput, ProxyTestKind, ProxyTestResult, now_secs,
};
use crate::state::AppState;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TEST_TARGET: &str = "https://opencode.ai/";

fn row_to_record(r: &crate::db::ProxyRow, url: String) -> ProxyRecord {
    ProxyRecord {
        id: r.id.clone(),
        name: r.name.clone(),
        url,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

pub async fn list(
    State(st): State<AppState>,
    _: ManagementAuth,
) -> Result<Json<Vec<ProxyRecord>>, ApiError> {
    let rows = st.db.list_proxies()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let url = st.decrypt_secret(&row.url_enc).await?.to_string();
        out.push(row_to_record(&row, url));
    }
    Ok(Json(out))
}

pub async fn create(
    State(st): State<AppState>,
    _: ManagementAuth,
    Json(input): Json<ProxyInput>,
) -> Result<(StatusCode, Json<ProxyRecord>), ApiError> {
    let name = input.name.trim().to_string();
    let url = input.url.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if st.db.get_proxy_by_name(&name)?.is_some() {
        return Err(ApiError::Conflict(format!("name already exists: {name}")));
    }
    // Validate the scheme early so a typo surfaces here, not at request time.
    crate::state::build_proxy_client(&url, std::time::Duration::from_secs(5))?;

    let id = Uuid::new_v4().to_string();
    let now = now_secs();
    let url_enc = st.encrypt_secret(&url).await?;
    st.db.insert_proxy(&id, &ProxyData { name, url_enc }, now)?;
    st.clear_proxy_client_cache();

    let row = st.db.get_proxy(&id)?.unwrap();
    let decrypted = st.decrypt_secret(&row.url_enc).await?.to_string();
    Ok((StatusCode::CREATED, Json(row_to_record(&row, decrypted))))
}

pub async fn update(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<ProxyInput>,
) -> Result<Json<ProxyRecord>, ApiError> {
    let existing = st
        .db
        .get_proxy(&id)?
        .ok_or_else(|| ApiError::NotFound("proxy not found".into()))?;
    let name = input.name.trim().to_string();
    let url = input.url.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if existing.name.to_lowercase() != name.to_lowercase() {
        if st.db.get_proxy_by_name(&name)?.is_some() {
            return Err(ApiError::Conflict(format!("name already exists: {name}")));
        }
    }
    crate::state::build_proxy_client(&url, std::time::Duration::from_secs(5))?;

    let url_enc = st.encrypt_secret(&url).await?;
    let row = st
        .db
        .update_proxy(&id, &ProxyData { name, url_enc }, now_secs())?
        .ok_or_else(|| ApiError::NotFound("proxy not found".into()))?;
    st.clear_proxy_client_cache();

    let decrypted = st.decrypt_secret(&row.url_enc).await?.to_string();
    Ok(Json(row_to_record(&row, decrypted)))
}

pub async fn delete(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
) -> Result<Json<OkResponse>, ApiError> {
    if !st.db.delete_proxy(&id)? {
        return Err(ApiError::NotFound("proxy not found".into()));
    }
    st.clear_proxy_client_cache();
    Ok(Json(OkResponse { ok: true }))
}

pub async fn test(
    State(st): State<AppState>,
    _: ManagementAuth,
    Path(id): Path<String>,
    Json(input): Json<ProxyTestInput>,
) -> Result<Json<ProxyTestResult>, ApiError> {
    let row = st
        .db
        .get_proxy(&id)?
        .ok_or_else(|| ApiError::NotFound("proxy not found".into()))?;
    let url = st.decrypt_secret(&row.url_enc).await?;
    let result = match input.kind {
        ProxyTestKind::Tcp => test_tcp(&url).await?,
        ProxyTestKind::Http => test_http(&url).await?,
    };
    Ok(Json(result))
}

async fn test_tcp(url: &str) -> Result<ProxyTestResult, ApiError> {
    let (host, port) = proxy_endpoint(url)?;
    let started = Instant::now();
    let connection = timeout(TEST_TIMEOUT, TcpStream::connect((host.as_str(), port))).await;
    let latency_ms = started.elapsed().as_millis() as i64;
    match connection {
        Ok(Ok(_)) => Ok(ProxyTestResult {
            kind: ProxyTestKind::Tcp,
            ok: true,
            latency_ms: Some(latency_ms),
            status: None,
            error: None,
        }),
        Ok(Err(error)) => Ok(ProxyTestResult {
            kind: ProxyTestKind::Tcp,
            ok: false,
            latency_ms: Some(latency_ms),
            status: None,
            error: Some(format!("TCP 连接失败: {error}")),
        }),
        Err(_) => Ok(ProxyTestResult {
            kind: ProxyTestKind::Tcp,
            ok: false,
            latency_ms: Some(latency_ms),
            status: None,
            error: Some("TCP 连接超时".into()),
        }),
    }
}

async fn test_http(url: &str) -> Result<ProxyTestResult, ApiError> {
    let client = crate::state::build_proxy_client(url, TEST_TIMEOUT)?;
    let started = Instant::now();
    let response = client.get(HTTP_TEST_TARGET).send().await;
    let latency_ms = started.elapsed().as_millis() as i64;
    match response {
        Ok(response) => {
            let status = response.status().as_u16();
            let ok = response.status().is_success();
            Ok(ProxyTestResult {
                kind: ProxyTestKind::Http,
                ok,
                latency_ms: Some(latency_ms),
                status: Some(status),
                error: (!ok).then(|| format!("HTTP 状态码 {status}")),
            })
        }
        Err(error) => Ok(ProxyTestResult {
            kind: ProxyTestKind::Http,
            ok: false,
            latency_ms: Some(latency_ms),
            status: None,
            error: Some(if error.is_timeout() {
                "HTTP 请求超时".into()
            } else {
                format!("HTTP 请求失败: {error}")
            }),
        }),
    }
}

fn proxy_endpoint(url: &str) -> Result<(String, u16), ApiError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| ApiError::BadRequest(format!("invalid proxy URL: {error}")))?;
    let host = parsed
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| ApiError::BadRequest("proxy URL is missing a host".into()))?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| ApiError::BadRequest("proxy URL is missing a port".into()))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::proxy_endpoint;

    #[test]
    fn extracts_proxy_host_and_port() {
        assert_eq!(
            proxy_endpoint("socks5h://user:pass@example.com:1080").unwrap(),
            ("example.com".to_string(), 1080)
        );
        assert_eq!(
            proxy_endpoint("http://127.0.0.1").unwrap(),
            ("127.0.0.1".to_string(), 80)
        );
    }

    #[test]
    fn rejects_invalid_proxy_url() {
        assert!(proxy_endpoint("not a url").is_err());
        assert!(proxy_endpoint("http://example.com").is_ok());
    }

    #[tokio::test]
    async fn tcp_test_reports_an_open_endpoint() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move { listener.accept().await.unwrap() });

        let result = super::test_tcp(&format!("http://{address}")).await.unwrap();

        assert!(result.ok);
        assert!(result.latency_ms.is_some_and(|latency| latency >= 0));
        accept.await.unwrap();
    }
}
