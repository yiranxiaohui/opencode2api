use axum::Json;
use axum::extract::State;
use serde_json::Value;
use uuid::Uuid;

use crate::db::{KeyData, ProxyData};
use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{
    ExportItem, ExportPayload, ImportResult, OPENCODE_BASE_URL, ProxyExport, now_secs,
};
use crate::state::AppState;

pub async fn export(
    State(st): State<AppState>,
    _: ManagementAuth,
) -> Result<Json<ExportPayload>, ApiError> {
    let mut proxies = Vec::new();
    for row in st.db.list_proxies()? {
        let url = st.decrypt_secret(&row.url_enc).await?.to_string();
        proxies.push(ProxyExport {
            name: row.name,
            url,
        });
    }
    let rows = st.db.all_key_rows()?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let api_key = st.decrypt_secret(&row.api_key_enc).await?;
        items.push(ExportItem {
            name: row.name,
            _base_url: None,
            api_key: api_key.to_string(),
            tags: row.tags,
            notes: row.notes,
            is_enabled: row.is_enabled,
            account_type: row.account_type,
            proxy: row.proxy_name,
        });
    }
    Ok(Json(ExportPayload { proxies, items }))
}

pub async fn import(
    State(st): State<AppState>,
    _: ManagementAuth,
    body: Json<Value>,
) -> Result<Json<ImportResult>, ApiError> {
    // Accept both the new `{proxies, items}` shape and the legacy bare array of
    // items (pre-proxy exports). Deserialize into a permissive struct.
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Body {
        New {
            #[serde(default)]
            proxies: Vec<ProxyExport>,
            items: Vec<ExportItem>,
        },
        Old(Vec<ExportItem>),
    }
    let parsed: Body = serde_json::from_value(body.0)
        .map_err(|_| ApiError::BadRequest("invalid export JSON".into()))?;
    let (proxies, items) = match parsed {
        Body::New { proxies, items } => (proxies, items),
        Body::Old(items) => (Vec::new(), items),
    };

    let mut imported = 0;
    let mut updated = 0;

    // Upsert proxies by name first so items below can link to them.
    for proxy in proxies {
        let name = proxy.name.trim().to_string();
        let url = proxy.url.trim().to_string();
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let url_enc = st.encrypt_secret(&url).await?;
        if let Some(existing) = st.db.get_proxy_by_name(&name)? {
            st.db
                .update_proxy(&existing.id, &ProxyData { name, url_enc }, now_secs())?;
        } else {
            st.db.insert_proxy(
                &Uuid::new_v4().to_string(),
                &ProxyData { name, url_enc },
                now_secs(),
            )?;
        }
    }

    for item in items {
        if item.name.trim().is_empty() {
            continue;
        }
        let proxy_id = match &item.proxy {
            Some(name) if !name.trim().is_empty() => {
                st.db.get_proxy_by_name(name.trim())?.map(|p| p.id)
            }
            _ => None,
        };
        let enc = st.encrypt_secret(&item.api_key).await?;
        let data = KeyData {
            name: item.name.trim().to_string(),
            base_url: OPENCODE_BASE_URL.to_string(),
            api_key_enc: enc,
            tags: item.tags,
            notes: item.notes,
            is_default: false,
            is_enabled: item.is_enabled,
            account_type: item.account_type,
            proxy_id,
            cookie_enc: None,
            workspace_id: None,
        };
        if st.db.get_key_by_name(&data.name)?.is_some() {
            if st.db.update_key_by_name(&data.name, &data, now_secs())? {
                updated += 1;
            }
        } else {
            st.db
                .insert_key(&Uuid::new_v4().to_string(), &data, now_secs())?;
            imported += 1;
        }
    }
    st.clear_proxy_client_cache();
    Ok(Json(ImportResult { imported, updated }))
}
