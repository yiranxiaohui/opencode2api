use std::time::Duration;

use crate::db::KeyRow;
use crate::error::ApiError;
use crate::models::{AccountType, AccountUsage, now_secs};
use crate::state::AppState;

const QUOTA_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Keep quota-exhausted Cookie accounts out of routing until a separate quota
/// query confirms that all reported quota windows have capacity again.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(QUOTA_REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = refresh_exhausted_accounts(&st).await {
                tracing::warn!("quota state refresh failed: {error}");
            }
        }
    });
}

async fn refresh_exhausted_accounts(st: &AppState) -> Result<(), ApiError> {
    let rows = st.db.all_key_rows()?;
    for row in rows.into_iter().filter(|row| {
        row.is_enabled
            && row.quota_exhausted_at.is_some()
            && row.cookie_enc.is_some()
            && row.workspace_id.is_some()
    }) {
        match refresh_account_usage(st, &row).await {
            Ok(usage) if usage.quota_available() == Some(true) => {
                tracing::info!(account_id = %row.id, account_name = %row.name, "quota recovered; account restored to routing");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(account_id = %row.id, account_name = %row.name, "quota refresh failed: {error}");
            }
        }
    }
    Ok(())
}

pub async fn refresh_account_usage(st: &AppState, row: &KeyRow) -> Result<AccountUsage, ApiError> {
    let cookie_enc = row.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("该账号不是通过 Cookie 导入，无法查询套餐额度".into())
    })?;
    let workspace = row
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::Internal("账号缺少 workspace".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let client = st.client_for_key(row).await?;
    let usage = crate::opencode_account::usage(&client, &cookie, workspace).await?;
    st.db.set_usage_cache(&row.id, &usage)?;
    if let Some(available) = usage.quota_available() {
        st.db.set_quota_exhausted(&row.id, !available, now_secs())?;
    }
    let account_type = if usage.plan_name.eq_ignore_ascii_case("OpenCode Go") {
        AccountType::Go
    } else {
        AccountType::Normal
    };
    st.db.set_account_type(&row.id, account_type, now_secs())?;
    Ok(usage)
}
