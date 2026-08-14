use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Client;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

use crate::db::{Db, KeyRow};
use crate::error::ApiError;

pub const ENCRYPTION_KEY_META: &str = "encryption_key";

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    /// Persisted AES-256 key used by the gateway independently of web sessions.
    pub master_key: Arc<RwLock<Option<Zeroizing<[u8; crate::crypto::KEY_LEN]>>>>,
    /// Client used by the proxy: redirects disabled (token leak), long read timeout.
    pub proxy_client: Client,
    /// Client used by connectivity tests: redirects disabled, short timeout.
    pub test_client: Client,
    /// Per-proxy clients (keyed by decrypted proxy URL) for the 600s forwarding
    /// path. reqwest requires the proxy to be set at client-build time, so we
    /// build once per URL and reuse. Purged on proxy create/update/delete.
    pub proxy_pool_clients: Arc<Mutex<HashMap<String, Client>>>,
    /// Account ids currently in quota-exhaustion cooldown, mapped to the Unix
    /// second at which the cooldown expires. In-memory only; cleared on restart.
    pub cooldowns: Arc<Mutex<HashMap<String, i64>>>,
    pub web_dist: PathBuf,
}

impl AppState {
    pub fn new(db_path: &Path, web_dist: PathBuf) -> Result<Self, ApiError> {
        let db = Arc::new(Db::open(db_path)?);
        let persisted_key = db
            .get_meta(ENCRYPTION_KEY_META)?
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
            cooldowns: Arc::new(Mutex::new(HashMap::new())),
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

    pub async fn client_for_proxy_id(&self, proxy_id: Option<&str>) -> Result<Client, ApiError> {
        let Some(proxy_id) = proxy_id else {
            return Ok(self.proxy_client.clone());
        };
        let proxy = self
            .db
            .get_proxy(proxy_id)?
            .ok_or_else(|| ApiError::BadRequest("proxy not found".into()))?;
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

    /// Mark an account as quota-exhausted for the cooldown window. Concurrent
    /// marks keep the longest remaining window (monotonic, idempotent).
    pub fn begin_cooldown(&self, id: &str) {
        let mut map = self.cooldowns.lock().unwrap();
        let until = crate::models::now_secs() + crate::routes::proxy::QUOTA_COOLDOWN_SECS;
        let entry = map.entry(id.to_string()).or_insert(until);
        *entry = (*entry).max(until);
    }

    /// True while `id` is inside its cooldown window at time `now` (Unix secs).
    pub fn in_quota_cooldown(&self, id: &str, now: i64) -> bool {
        self.cooldowns
            .lock()
            .unwrap()
            .get(id)
            .is_some_and(|until| now < *until)
    }

    /// Return the cooldown deadline when it is still active. Expired entries
    /// are removed opportunistically so the in-memory map stays bounded.
    pub fn quota_cooldown_until(&self, id: &str, now: i64) -> Option<i64> {
        let mut map = self.cooldowns.lock().unwrap();
        match map.get(id).copied() {
            Some(until) if now < until => Some(until),
            Some(_) => {
                map.remove(id);
                None
            }
            None => None,
        }
    }

    pub async fn decrypt_secret(&self, enc: &str) -> Result<Zeroizing<String>, ApiError> {
        let guard = self.master_key.read().await;
        let key = guard.as_ref().ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "encryption key unavailable; log in once to restore service".into(),
            )
        })?;
        let pt = crate::crypto::decrypt(key, enc)?;
        let s = String::from_utf8(pt.to_vec())
            .map_err(|_| ApiError::Internal("secret is not valid utf-8".into()))?;
        Ok(Zeroizing::new(s))
    }

    pub async fn encrypt_secret(&self, plain: &str) -> Result<String, ApiError> {
        let guard = self.master_key.read().await;
        let key = guard.as_ref().ok_or_else(|| {
            ApiError::ServiceUnavailable(
                "encryption key unavailable; log in once to restore service".into(),
            )
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quota_cooldown_marks_and_expires() {
        let path = std::env::temp_dir().join(format!("oc2a-state-{}.db", uuid::Uuid::new_v4()));
        crate::migration::run(&path).await.unwrap();
        let st = AppState::new(&path, PathBuf::from("frontend/dist")).unwrap();
        let now = crate::models::now_secs();
        assert!(!st.in_quota_cooldown("a", now));
        st.begin_cooldown("a");
        assert!(st.in_quota_cooldown("a", now));
        assert!(st.quota_cooldown_until("a", now).is_some());
        assert!(!st.in_quota_cooldown("a", now + crate::routes::proxy::QUOTA_COOLDOWN_SECS + 1));
        assert_eq!(
            st.quota_cooldown_until("a", now + crate::routes::proxy::QUOTA_COOLDOWN_SECS + 1),
            None
        );
        assert!(!st.in_quota_cooldown("b", now));
        let _ = std::fs::remove_file(path);
    }
}
