use serde::{Deserialize, Serialize};

pub const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen/go/v1";
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    #[default]
    Normal,
    Go,
}

impl AccountType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Go => "go",
        }
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyInput {
    pub name: String,
    /// Kept for backwards-compatible clients; OpenCode's official URL is used.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>, // required on create, optional on update (keep existing)
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub account_type: Option<AccountType>,
    #[serde(default)]
    pub is_default: bool,
    /// Attached forward proxy (HTTP/SOCKS5) from the pool, if any.
    /// Outer `None` = field omitted (keep existing on update); inner `None` = explicitly no proxy.
    #[serde(default)]
    pub proxy_id: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeySummary {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub tags: Vec<String>,
    pub notes: String,
    pub is_enabled: bool,
    pub account_type: AccountType,
    /// Unix timestamp when routing was stopped after a confirmed quota failure.
    pub quota_exhausted_at: Option<i64>,
    pub model_count: usize,
    pub created_at: i64,
    pub updated_at: i64,
    pub proxy_id: Option<String>,
    pub proxy_name: Option<String>,
    pub has_cookie: bool,
    pub workspace_id: Option<String>,
    pub usage_cache: Option<AccountUsage>,
}

impl KeySummary {
    pub fn from_row(r: &crate::db::KeyRow) -> Self {
        KeySummary {
            id: r.id.clone(),
            name: r.name.clone(),
            base_url: OPENCODE_BASE_URL.to_string(),
            tags: r.tags.clone(),
            notes: r.notes.clone(),
            is_enabled: r.is_enabled,
            account_type: r.account_type,
            quota_exhausted_at: r.quota_exhausted_at,
            model_count: r.model_cache.len(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            proxy_id: r.proxy_id.clone(),
            proxy_name: r.proxy_name.clone(),
            has_cookie: r.cookie_enc.is_some(),
            workspace_id: r.workspace_id.clone(),
            usage_cache: r.usage_cache.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KeyRecord {
    #[serde(flatten)]
    pub summary: KeySummary,
    pub api_key: String,
    pub model_cache: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub owned_by: String,
}

#[derive(Debug, Serialize)]
pub struct ManagedModel {
    pub id: String,
    pub owned_by: String,
    pub account_count: usize,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct ModelEnabledInput {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportItem {
    pub name: String,
    /// Accepted when importing old backups, but ignored by the gateway.
    #[serde(default, rename = "base_url", skip_serializing)]
    pub _base_url: Option<String>,
    pub api_key: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_true")]
    pub is_enabled: bool,
    #[serde(default)]
    pub account_type: AccountType,
    /// Name of the attached proxy (matched by name on import).
    #[serde(default)]
    pub proxy: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct KeyEnabledInput {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct CookieImportInput {
    pub cookie: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub account_type: AccountType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageWindow {
    pub usage_percent: f64,
    pub remaining_percent: f64,
    pub reset_in_sec: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUsage {
    pub plan_name: String,
    pub plan_status: String,
    pub region: Option<String>,
    pub balance_microcents: Option<i64>,
    pub monthly_limit_microcents: Option<i64>,
    pub monthly_usage_microcents: Option<i64>,
    pub rolling: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    pub monthly: Option<UsageWindow>,
    pub fetched_at: i64,
}

impl AccountUsage {
    /// `None` means the upstream response contained no recognized quota window,
    /// so an existing routing state must be preserved rather than guessed.
    pub fn quota_available(&self) -> Option<bool> {
        let remaining = [
            self.rolling.as_ref().map(|window| window.remaining_percent),
            self.weekly.as_ref().map(|window| window.remaining_percent),
            self.monthly.as_ref().map(|window| window.remaining_percent),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !remaining.is_empty() {
            return Some(remaining.into_iter().all(|value| value > 0.0));
        }
        if let (Some(limit), Some(used)) =
            (self.monthly_limit_microcents, self.monthly_usage_microcents)
            && limit > 0
        {
            return Some(used < limit);
        }
        self.balance_microcents.map(|balance| balance > 0)
    }
}

#[cfg(test)]
mod account_usage_tests {
    use super::{AccountUsage, UsageWindow};

    fn usage(remaining: &[f64]) -> AccountUsage {
        let mut windows = remaining.iter().map(|value| UsageWindow {
            usage_percent: 100.0 - value,
            remaining_percent: *value,
            reset_in_sec: 0,
            status: "ok".into(),
        });
        AccountUsage {
            plan_name: "test".into(),
            plan_status: "active".into(),
            region: None,
            balance_microcents: None,
            monthly_limit_microcents: None,
            monthly_usage_microcents: None,
            rolling: windows.next(),
            weekly: windows.next(),
            monthly: windows.next(),
            fetched_at: 0,
        }
    }

    #[test]
    fn quota_requires_every_reported_window_to_have_capacity() {
        assert_eq!(usage(&[25.0, 10.0]).quota_available(), Some(true));
        assert_eq!(usage(&[25.0, 0.0]).quota_available(), Some(false));
        assert_eq!(usage(&[]).quota_available(), None);
    }

    #[test]
    fn quota_falls_back_to_billing_capacity_when_windows_are_absent() {
        let mut value = usage(&[]);
        value.monthly_limit_microcents = Some(1_000);
        value.monthly_usage_microcents = Some(1_000);
        assert_eq!(value.quota_available(), Some(false));

        value.monthly_usage_microcents = Some(900);
        assert_eq!(value.quota_available(), Some(true));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteLinkResult {
    pub account_id: String,
    pub account_name: String,
    pub invite_link: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InviteReward {
    pub id: String,
    pub source: String,
    pub status: String,
    pub email: String,
    pub amount_cents: i64,
    pub created_at: Option<String>,
    pub claimable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteRewardsResult {
    pub account_id: String,
    pub account_name: String,
    pub rewards: Vec<InviteReward>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteRewardClaimResult {
    pub account_id: String,
    pub account_name: String,
    pub reward_id: String,
    pub amount_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyExport {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportPayload {
    pub proxies: Vec<ProxyExport>,
    pub items: Vec<ExportItem>,
}

#[derive(Debug, Deserialize)]
pub struct PasswordBody {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordBody {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyInput {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyRecord {
    pub id: String,
    pub name: String,
    pub url: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub installed: bool,
    pub logged_in: bool,
    pub key_count: i64,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: Option<u128>,
    pub models: Vec<ModelInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub imported: usize,
    pub updated: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientKeyInput {
    pub name: String,
    /// `null`/omitted grants access to every globally enabled model.
    pub allowed_models: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientKeyModelsInput {
    /// `null` grants access to every globally enabled model.
    pub allowed_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientKeySummary {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    /// Decrypted only for authenticated management requests. Legacy keys are unavailable.
    pub api_key: Option<String>,
    /// `null` grants access to every globally enabled model.
    pub allowed_models: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ClientKeyCreated {
    #[serde(flatten)]
    pub summary: ClientKeySummary,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminTokenInput {
    pub name: String,
    pub password: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeAdminTokenInput {
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminTokenSummary {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdminTokenCreated {
    #[serde(flatten)]
    pub summary: AdminTokenSummary,
    pub token: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestLogRecord {
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
    pub status: u16,
    pub latency_ms: i64,
    pub first_token_ms: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub error: Option<String>,
}

impl RequestLogRecord {
    pub fn from_row(r: &crate::db::RequestLogRow) -> Self {
        RequestLogRecord {
            id: r.id.clone(),
            created_at: r.created_at,
            client_key_id: r.client_key_id.clone(),
            client_key_name: r.client_key_name.clone(),
            route_key_id: r.route_key_id.clone(),
            route_key_name: r.route_key_name.clone(),
            method: r.method.clone(),
            path: r.path.clone(),
            model: r.model.clone(),
            stream: r.stream,
            status: r.status as u16,
            latency_ms: r.latency_ms,
            first_token_ms: r.first_token_ms,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            cached_tokens: r.cached_tokens,
            cache_creation_tokens: r.cache_creation_tokens,
            error: r.error.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LogListResponse {
    pub items: Vec<RequestLogRecord>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogStatsTotals {
    pub total_calls: i64,
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_cached_tokens: i64,
    pub total_cache_creation_tokens: i64,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogStatsGroup {
    pub name: String,
    pub calls: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cache_creation_tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct LogStatsResponse {
    pub totals: LogStatsTotals,
    pub by_model: Vec<LogStatsGroup>,
    pub by_client: Vec<LogStatsGroup>,
}
