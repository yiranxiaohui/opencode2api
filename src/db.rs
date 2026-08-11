use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

use crate::error::ApiError;
use crate::models::{LogStatsGroup, LogStatsTotals, ModelInfo, now_secs};

const COLUMNS: &str = "id, name, base_url, api_key_enc, tags, notes, model_cache, is_default, created_at, updated_at, proxy_id, is_enabled";

/// Qualify every `api_keys` column for queries that LEFT JOIN `proxies`
/// (`id`, `name`, `created_at`, `updated_at` exist in both tables).
const KEY_SELECT: &str = "api_keys.id, api_keys.name, api_keys.base_url, api_keys.api_key_enc, api_keys.tags, \
     api_keys.notes, api_keys.model_cache, api_keys.is_default, api_keys.created_at, \
     api_keys.updated_at, api_keys.proxy_id, api_keys.is_enabled";

/// Fields that can be written for a key. `api_key_enc` is the already-encrypted blob.
#[derive(Debug, Clone)]
pub struct KeyData {
    pub name: String,
    pub base_url: String,
    pub api_key_enc: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub is_default: bool,
    pub is_enabled: bool,
    pub proxy_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KeyRow {
    pub id: String,
    pub name: String,
    /// Retained for database compatibility; routing uses the fixed OpenCode URL.
    pub _base_url: String,
    pub api_key_enc: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub model_cache: Vec<ModelInfo>,
    pub is_default: bool,
    pub is_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub proxy_id: Option<String>,
    pub proxy_name: Option<String>,
}

/// Fields that can be written for a proxy. `url_enc` is the already-encrypted URL.
#[derive(Debug, Clone)]
pub struct ProxyData {
    pub name: String,
    pub url_enc: String,
}

#[derive(Debug, Clone)]
pub struct ProxyRow {
    pub id: String,
    pub name: String,
    pub url_enc: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct ClientKeyRow {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub key_enc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequestLogRow {
    pub id: String,
    pub created_at: i64,
    pub client_key_id: Option<String>,
    pub client_key_name: String,
    pub route_key_id: Option<String>,
    pub route_key_name: Option<String>,
    pub method: String,
    pub path: String,
    pub model: Option<String>,
    pub stream: bool,
    pub status: i64,
    pub latency_ms: i64,
    pub first_token_ms: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub error: Option<String>,
}

/// Filter for `list_request_logs`. Empty fields are ignored.
#[derive(Debug, Default)]
pub struct LogFilter {
    pub client_key_id: Option<String>,
    pub route_key_id: Option<String>,
    pub model: Option<String>,
    pub status: Option<i64>,
    pub limit: i64,
    pub offset: i64,
}

fn row_to_key(r: &rusqlite::Row) -> rusqlite::Result<KeyRow> {
    let tags_json: String = r.get(4)?;
    let models_json: String = r.get(6)?;
    Ok(KeyRow {
        id: r.get(0)?,
        name: r.get(1)?,
        _base_url: r.get(2)?,
        api_key_enc: r.get(3)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        notes: r.get(5)?,
        model_cache: serde_json::from_str(&models_json).unwrap_or_default(),
        is_default: r.get::<_, i64>(7)? != 0,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        proxy_id: r.get(10)?,
        is_enabled: r.get::<_, i64>(11)? != 0,
        proxy_name: r.get(12)?,
    })
}

pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        Ok(Db(Mutex::new(conn)))
    }

    // ---- meta ---------------------------------------------------------------

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(ApiError::from)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- client API keys ---------------------------------------------------

    pub fn list_client_keys(&self) -> Result<Vec<ClientKeyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, prefix, created_at, last_used_at, key_enc
             FROM client_api_keys ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ClientKeyRow {
                id: r.get(0)?,
                name: r.get(1)?,
                prefix: r.get(2)?,
                created_at: r.get(3)?,
                last_used_at: r.get(4)?,
                key_enc: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn insert_client_key(
        &self,
        id: &str,
        name: &str,
        key_hash: &str,
        key_enc: &str,
        prefix: &str,
        created_at: i64,
    ) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO client_api_keys(id, name, key_hash, key_enc, prefix, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, name, key_hash, key_enc, prefix, created_at],
        )?;
        Ok(())
    }

    pub fn delete_client_key(&self, id: &str) -> Result<bool, ApiError> {
        let conn = self.0.lock().unwrap();
        Ok(conn.execute("DELETE FROM client_api_keys WHERE id = ?1", params![id])? > 0)
    }

    pub fn client_key_by_hash(&self, key_hash: &str) -> Result<Option<ClientKeyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT id, name, prefix, created_at, last_used_at, key_enc
             FROM client_api_keys WHERE key_hash = ?1",
            params![key_hash],
            |r| {
                Ok(ClientKeyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    prefix: r.get(2)?,
                    created_at: r.get(3)?,
                    last_used_at: r.get(4)?,
                    key_enc: r.get(5)?,
                })
            },
        )
        .optional()
        .map_err(ApiError::from)
    }

    /// Bump `last_used_at` on successful auth.
    pub fn touch_client_key(&self, key_hash: &str, now: i64) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE client_api_keys SET last_used_at = ?2 WHERE key_hash = ?1",
            params![key_hash, now],
        )?;
        Ok(())
    }

    // ---- proxies -----------------------------------------------------------

    pub fn list_proxies(&self) -> Result<Vec<ProxyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, url_enc, created_at, updated_at
             FROM proxies ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProxyRow {
                id: r.get(0)?,
                name: r.get(1)?,
                url_enc: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_proxy(&self, id: &str) -> Result<Option<ProxyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT id, name, url_enc, created_at, updated_at
             FROM proxies WHERE id = ?1",
            params![id],
            |r| {
                Ok(ProxyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    url_enc: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(ApiError::from)
    }

    pub fn get_proxy_by_name(&self, name: &str) -> Result<Option<ProxyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT id, name, url_enc, created_at, updated_at
             FROM proxies WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| {
                Ok(ProxyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    url_enc: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            },
        )
        .optional()
        .map_err(ApiError::from)
    }

    pub fn insert_proxy(&self, id: &str, data: &ProxyData, now: i64) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO proxies(id, name, url_enc, created_at, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?4)",
            params![id, data.name, data.url_enc, now],
        )?;
        Ok(())
    }

    pub fn update_proxy(
        &self,
        id: &str,
        data: &ProxyData,
        now: i64,
    ) -> Result<Option<ProxyRow>, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE proxies SET name=?2, url_enc=?3, updated_at=?4 WHERE id=?1",
            params![id, data.name, data.url_enc, now],
        )?;
        if n == 0 {
            return Ok(None);
        }
        tx.commit()?;
        drop(conn);
        self.get_proxy(id)
    }

    /// Delete a proxy and detach it from every key.
    pub fn delete_proxy(&self, id: &str) -> Result<bool, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE api_keys SET proxy_id = NULL WHERE proxy_id = ?1",
            params![id],
        )?;
        let n = tx.execute("DELETE FROM proxies WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    // ---- counts / lists -----------------------------------------------------

    pub fn key_count(&self) -> Result<i64, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM api_keys", [], |r| r.get(0))
            .map_err(ApiError::from)
    }

    pub fn list_keys(&self) -> Result<Vec<KeyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {KEY_SELECT}, p.name FROM api_keys
             LEFT JOIN proxies p ON p.id = api_keys.proxy_id
             ORDER BY api_keys.is_default DESC, api_keys.created_at DESC"
        ))?;
        let rows = stmt.query_map([], row_to_key)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn all_key_rows(&self) -> Result<Vec<KeyRow>, ApiError> {
        self.list_keys()
    }

    // ---- single lookups -----------------------------------------------------

    pub fn get_key(&self, id: &str) -> Result<Option<KeyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            &format!(
                "SELECT {KEY_SELECT}, p.name FROM api_keys
                 LEFT JOIN proxies p ON p.id = api_keys.proxy_id WHERE api_keys.id = ?1"
            ),
            params![id],
            row_to_key,
        )
        .optional()
        .map_err(ApiError::from)
    }

    pub fn get_key_by_name(&self, name: &str) -> Result<Option<KeyRow>, ApiError> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            &format!(
                "SELECT {KEY_SELECT}, p.name FROM api_keys
                 LEFT JOIN proxies p ON p.id = api_keys.proxy_id WHERE api_keys.name = ?1 COLLATE NOCASE"
            ),
            params![name],
            row_to_key,
        )
        .optional()
        .map_err(ApiError::from)
    }

    // ---- writes -------------------------------------------------------------

    pub fn insert_key(&self, id: &str, data: &KeyData, now: i64) -> Result<(), ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        if data.is_default {
            tx.execute("UPDATE api_keys SET is_default = 0", [])?;
        }
        tx.execute(
            &format!(
                "INSERT INTO api_keys ({COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,'[]',?7,?8,?8,?9,?10)"
            ),
            params![
                id,
                data.name,
                data.base_url,
                data.api_key_enc,
                serde_json::to_string(&data.tags)?,
                data.notes,
                data.is_default as i64,
                now,
                data.proxy_id,
                data.is_enabled as i64,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn update_key(
        &self,
        id: &str,
        data: &KeyData,
        now: i64,
    ) -> Result<Option<KeyRow>, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let exists: i64 = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?1)",
            params![id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Ok(None);
        }
        if data.is_default {
            tx.execute("UPDATE api_keys SET is_default = 0", [])?;
        }
        tx.execute(
            &format!(
                "UPDATE api_keys SET name=?2, base_url=?3, api_key_enc=?4, tags=?5, notes=?6,
                 is_default=?7, updated_at=?8, proxy_id=?9, is_enabled=?10 WHERE id=?1"
            ),
            params![
                id,
                data.name,
                data.base_url,
                data.api_key_enc,
                serde_json::to_string(&data.tags)?,
                data.notes,
                data.is_default as i64,
                now,
                data.proxy_id,
                data.is_enabled as i64,
            ],
        )?;
        tx.commit()?;
        drop(conn);
        self.get_key(id)
    }

    pub fn update_key_by_name(
        &self,
        name: &str,
        data: &KeyData,
        now: i64,
    ) -> Result<bool, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE api_keys SET base_url=?2, api_key_enc=?3, tags=?4, notes=?5, updated_at=?6, proxy_id=?7, is_enabled=?8
             WHERE name=?1 COLLATE NOCASE",
            params![
                name,
                data.base_url,
                data.api_key_enc,
                serde_json::to_string(&data.tags)?,
                data.notes,
                now,
                data.proxy_id,
                data.is_enabled as i64
            ],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    pub fn delete_key(&self, id: &str) -> Result<bool, ApiError> {
        let conn = self.0.lock().unwrap();
        let n = conn.execute("DELETE FROM api_keys WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn set_default(&self, id: &str, now: i64) -> Result<bool, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let exists: i64 = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ?1 AND is_enabled = 1)",
            params![id],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Ok(false);
        }
        tx.execute("UPDATE api_keys SET is_default = 0", [])?;
        tx.execute(
            "UPDATE api_keys SET is_default = 1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn set_key_enabled(&self, id: &str, enabled: bool, now: i64) -> Result<bool, ApiError> {
        let conn = self.0.lock().unwrap();
        let changed = conn.execute(
            "UPDATE api_keys
             SET is_enabled = ?2,
                 is_default = CASE WHEN ?2 = 0 THEN 0 ELSE is_default END,
                 updated_at = ?3
             WHERE id = ?1",
            params![id, enabled as i64, now],
        )?;
        Ok(changed > 0)
    }

    pub fn set_model_cache(
        &self,
        id: &str,
        models: &[ModelInfo],
        now: i64,
    ) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE api_keys SET model_cache = ?1, updated_at = ?2 WHERE id = ?3",
            params![serde_json::to_string(models)?, now, id],
        )?;
        Ok(())
    }

    pub fn reencrypt_all(
        &self,
        old_key: &[u8; crate::crypto::KEY_LEN],
        new_key: &[u8; crate::crypto::KEY_LEN],
    ) -> Result<usize, ApiError> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, api_key_enc FROM api_keys")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut count = 0;
        for (id, enc) in rows {
            let pt = crate::crypto::decrypt(old_key, &enc)?;
            let new_enc = crate::crypto::encrypt(new_key, &pt)?;
            tx.execute(
                "UPDATE api_keys SET api_key_enc = ?1 WHERE id = ?2",
                params![new_enc, id],
            )?;
            count += 1;
        }
        // Proxy URLs may embed credentials; re-encrypt them with the same key.
        let proxies: Vec<(String, String)> = {
            let mut stmt = tx.prepare("SELECT id, url_enc FROM proxies")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, enc) in proxies {
            let pt = crate::crypto::decrypt(old_key, &enc)?;
            let new_enc = crate::crypto::encrypt(new_key, &pt)?;
            tx.execute(
                "UPDATE proxies SET url_enc = ?1 WHERE id = ?2",
                params![new_enc, id],
            )?;
            count += 1;
        }
        // Newly created client credentials are recoverable from the management
        // page, so rotate their encrypted copies as well. Legacy rows are NULL.
        let client_keys: Vec<(String, String)> = {
            let mut stmt =
                tx.prepare("SELECT id, key_enc FROM client_api_keys WHERE key_enc IS NOT NULL")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, enc) in client_keys {
            let pt = crate::crypto::decrypt(old_key, &enc)?;
            let new_enc = crate::crypto::encrypt(new_key, &pt)?;
            tx.execute(
                "UPDATE client_api_keys SET key_enc = ?1 WHERE id = ?2",
                params![new_enc, id],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    #[allow(dead_code)]
    pub fn touch(&self, id: &str) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE api_keys SET updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    // ---- request logs ------------------------------------------------------

    pub fn insert_request_log(&self, row: &RequestLogRow) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO request_logs
               (id, created_at, client_key_id, client_key_name, route_key_id, route_key_name,
                method, path, model, stream, status, latency_ms, first_token_ms,
                prompt_tokens, completion_tokens, cached_tokens, cache_creation_tokens, error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                row.id,
                row.created_at,
                row.client_key_id,
                row.client_key_name,
                row.route_key_id,
                row.route_key_name,
                row.method,
                row.path,
                row.model,
                row.stream as i64,
                row.status,
                row.latency_ms,
                row.first_token_ms,
                row.prompt_tokens,
                row.completion_tokens,
                row.cached_tokens,
                row.cache_creation_tokens,
                row.error,
            ],
        )?;
        Ok(())
    }

    /// Finalize timing and usage after a streaming response has ended.
    pub fn finalize_stream_log(
        &self,
        id: &str,
        first_token_ms: Option<i64>,
        total_ms: i64,
        prompt_tokens: Option<i64>,
        completion_tokens: Option<i64>,
        cached_tokens: Option<i64>,
        cache_creation_tokens: Option<i64>,
    ) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE request_logs
             SET first_token_ms = COALESCE(?2, first_token_ms),
                 latency_ms = ?3,
                 prompt_tokens = COALESCE(?4, prompt_tokens),
                 completion_tokens = COALESCE(?5, completion_tokens),
                 cached_tokens = COALESCE(?6, cached_tokens),
                 cache_creation_tokens = COALESCE(?7, cache_creation_tokens)
             WHERE id = ?1",
            params![
                id,
                first_token_ms,
                total_ms,
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                cache_creation_tokens
            ],
        )?;
        Ok(())
    }

    pub fn list_request_logs(&self, f: &LogFilter) -> Result<(Vec<RequestLogRow>, i64), ApiError> {
        let (where_sql, vals) = build_log_where(f);
        let sql_refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| v.as_ref()).collect();

        let conn = self.0.lock().unwrap();
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs{where_sql}"),
            params_from_iter(sql_refs.iter().copied()),
            |r| r.get(0),
        )?;

        let mut qp: Vec<&dyn rusqlite::ToSql> = sql_refs;
        let limit = f.limit;
        let offset = f.offset;
        qp.push(&limit);
        qp.push(&offset);
        let mut stmt = conn.prepare(&format!(
            "SELECT id, created_at, client_key_id, client_key_name, route_key_id, route_key_name,
                    method, path, model, stream, status, latency_ms, first_token_ms,
                    prompt_tokens, completion_tokens, cached_tokens, cache_creation_tokens, error
             FROM request_logs{where_sql}
             ORDER BY created_at DESC, rowid DESC
             LIMIT ? OFFSET ?"
        ))?;
        let rows = stmt.query_map(params_from_iter(qp), |r| {
            Ok(RequestLogRow {
                id: r.get(0)?,
                created_at: r.get(1)?,
                client_key_id: r.get(2)?,
                client_key_name: r.get(3)?,
                route_key_id: r.get(4)?,
                route_key_name: r.get(5)?,
                method: r.get(6)?,
                path: r.get(7)?,
                model: r.get(8)?,
                stream: r.get::<_, i64>(9)? != 0,
                status: r.get(10)?,
                latency_ms: r.get(11)?,
                first_token_ms: r.get(12)?,
                prompt_tokens: r.get(13)?,
                completion_tokens: r.get(14)?,
                cached_tokens: r.get(15)?,
                cache_creation_tokens: r.get(16)?,
                error: r.get(17)?,
            })
        })?;
        let items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((items, total))
    }

    /// Aggregated usage across every log row matching `f`. Totals plus the
    /// top models and top client keys by token volume.
    pub fn log_stats(
        &self,
        f: &LogFilter,
    ) -> Result<(LogStatsTotals, Vec<LogStatsGroup>, Vec<LogStatsGroup>), ApiError> {
        let (where_sql, vals) = build_log_where(f);
        let sql_refs: Vec<&dyn rusqlite::ToSql> = vals.iter().map(|v| v.as_ref()).collect();

        let conn = self.0.lock().unwrap();
        let totals = conn.query_row(
            &format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(prompt_tokens), 0),
                        COALESCE(SUM(completion_tokens), 0),
                        COALESCE(SUM(cached_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(latency_ms), 0)
                 FROM request_logs{where_sql}"
            ),
            params_from_iter(sql_refs.iter().copied()),
            |r| {
                Ok(LogStatsTotals {
                    total_calls: r.get(0)?,
                    total_prompt_tokens: r.get(1)?,
                    total_completion_tokens: r.get(2)?,
                    total_cached_tokens: r.get(3)?,
                    total_cache_creation_tokens: r.get(4)?,
                    total_duration_ms: r.get(5)?,
                })
            },
        )?;

        let by_model = log_group_stats(&conn, &where_sql, &sql_refs, "model", "model")?;
        let by_client = log_group_stats(
            &conn,
            &where_sql,
            &sql_refs,
            "client_key_name",
            "client_key_name",
        )?;
        Ok((totals, by_model, by_client))
    }

    pub fn clear_request_logs(&self) -> Result<(), ApiError> {
        let conn = self.0.lock().unwrap();
        conn.execute("DELETE FROM request_logs", [])?;
        Ok(())
    }
}

/// Shared `WHERE` clause + bound params for `request_logs` queries.
fn build_log_where(f: &LogFilter) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut conds: Vec<String> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(id) = &f.client_key_id {
        conds.push("client_key_id = ?".into());
        vals.push(Box::new(id.clone()));
    }
    if let Some(id) = &f.route_key_id {
        conds.push("route_key_id = ?".into());
        vals.push(Box::new(id.clone()));
    }
    if let Some(model) = &f.model {
        conds.push("model LIKE ?".into());
        vals.push(Box::new(format!("%{model}%")));
    }
    if let Some(status) = f.status {
        conds.push("status = ?".into());
        vals.push(Box::new(status));
    }
    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    (where_sql, vals)
}

/// Group `request_logs` by a column and return the top `LIMIT` groups ordered
/// by combined token usage.
fn log_group_stats(
    conn: &Connection,
    where_sql: &str,
    sql_refs: &[&dyn rusqlite::ToSql],
    label_col: &str,
    group_col: &str,
) -> Result<Vec<LogStatsGroup>, ApiError> {
    const LIMIT: i64 = 10;
    let mut qp: Vec<&dyn rusqlite::ToSql> = sql_refs.to_vec();
    qp.push(&LIMIT);
    let mut stmt = conn.prepare(&format!(
        "SELECT {label_col}, COUNT(*),
                COALESCE(SUM(prompt_tokens), 0),
                COALESCE(SUM(completion_tokens), 0),
                COALESCE(SUM(cached_tokens), 0),
                COALESCE(SUM(cache_creation_tokens), 0)
         FROM request_logs{where_sql}
         GROUP BY {group_col}
         ORDER BY (COALESCE(SUM(prompt_tokens), 0) + COALESCE(SUM(completion_tokens), 0)) DESC
         LIMIT ?"
    ))?;
    let rows = stmt.query_map(params_from_iter(qp), |r| {
        Ok(LogStatsGroup {
            name: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            calls: r.get(1)?,
            prompt_tokens: r.get(2)?,
            completion_tokens: r.get(3)?,
            cached_tokens: r.get(4)?,
            cache_creation_tokens: r.get(5)?,
        })
    })?;
    let groups = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(groups)
}
