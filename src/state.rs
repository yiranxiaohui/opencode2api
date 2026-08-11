use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

use crate::db::{Db, KeyRow};
use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    /// AES-256 key derived from the login password and restored on startup.
    pub master_key: Arc<RwLock<Option<Zeroizing<[u8; crate::crypto::KEY_LEN]>>>>,
    /// Client used by the proxy: redirects disabled (token leak), long read timeout.
    pub proxy_client: Client,
    /// Client used by connectivity tests: redirects disabled, short timeout.
    pub test_client: Client,
    /// Per-proxy clients (keyed by decrypted proxy URL) for the 600s forwarding
    /// path. reqwest requires the proxy to be set at client-build time, so we
    /// build once per URL and reuse. Purged on proxy create/update/delete.
    pub proxy_pool_clients: Arc<Mutex<HashMap<String, Client>>>,
    /// Monotonic cursor shared by every cloned state for round-robin account routing.
    pub account_cursor: Arc<AtomicU64>,
    pub web_dist: PathBuf,
}

impl AppState {
    pub fn new(db_path: &Path, web_dist: PathBuf) -> Result<Self, ApiError> {
        let db = Arc::new(Db::open(db_path)?);
        let persisted_key = db
            .get_meta("auto_unlock_key")?
            .map(|encoded| crate::crypto::decode_master_key(&encoded))
            .transpose()?;
        let proxy_client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| ApiError::Internal(format!("proxy client: {e}")))?;
        let test_client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| ApiError::Internal(format!("test client: {e}")))?;
        Ok(Self {
            db,
            master_key: Arc::new(RwLock::new(persisted_key)),
            proxy_client,
            test_client,
            proxy_pool_clients: Arc::new(Mutex::new(HashMap::new())),
            account_cursor: Arc::new(AtomicU64::new(0)),
            web_dist,
        })
    }

    /// Client for forwarding upstream requests, honoring the key's attached
    /// forward proxy (HTTP/SOCKS5) if it has one.
    pub async fn client_for_key(&self, row: &KeyRow) -> Result<Client, ApiError> {
        let Some(proxy_id) = &row.proxy_id else {
            return Ok(self.proxy_client.clone());
        };
        let proxy = self
            .db
            .get_proxy(proxy_id)?
            .ok_or_else(|| ApiError::Internal("attached proxy not found".into()))?;
        let url = self.decrypt_secret(&proxy.url_enc).await?;
        self.cached_proxy_client(&url)
    }

    /// Reuse a per-URL client when possible; otherwise build one with the same
    /// long-timeout settings as `proxy_client`.
    fn cached_proxy_client(&self, url: &str) -> Result<Client, ApiError> {
        let mut guard = self.proxy_pool_clients.lock().unwrap();
        if let Some(client) = guard.get(url) {
            return Ok(client.clone());
        }
        let client = build_proxy_client(url, Duration::from_secs(600))?;
        guard.insert(url.to_string(), client.clone());
        Ok(client)
    }

    /// Drop all cached per-proxy clients (after proxy create/update/delete).
    pub fn clear_proxy_client_cache(&self) {
        self.proxy_pool_clients.lock().unwrap().clear();
    }

    pub async fn decrypt_secret(&self, enc: &str) -> Result<Zeroizing<String>, ApiError> {
        let guard = self.master_key.read().await;
        let key = guard.as_ref().ok_or(ApiError::Locked)?;
        let pt = crate::crypto::decrypt(key, enc)?;
        let s = String::from_utf8(pt.to_vec())
            .map_err(|_| ApiError::Internal("secret is not valid utf-8".into()))?;
        Ok(Zeroizing::new(s))
    }

    pub async fn encrypt_secret(&self, plain: &str) -> Result<String, ApiError> {
        let guard = self.master_key.read().await;
        let key = guard.as_ref().ok_or(ApiError::Locked)?;
        crate::crypto::encrypt(key, plain.as_bytes())
    }
}

/// Build a reqwest client that routes every request through the given proxy
/// URL (`http://`, `https://`, `socks5://`, …). Redirects stay disabled so a
/// Bearer token can never leak to a third-party host.
pub fn build_proxy_client(url: &str, timeout: Duration) -> Result<Client, ApiError> {
    let proxy = reqwest::Proxy::all(url)
        .map_err(|e| ApiError::BadRequest(format!("invalid proxy URL: {e}")))?;
    Client::builder()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|e| ApiError::Internal(format!("proxy client: {e}")))
}
