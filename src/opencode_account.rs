use regex::Regex;
use reqwest::{Client, header};
use std::collections::HashSet;
use std::time::Duration;

use crate::error::ApiError;
use crate::models::{AccountUsage, InviteReward, UsageWindow, now_secs};

const ORIGIN: &str = "https://opencode.ai";
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/136 Safari/537.36";
const SUBSCRIPTION_QUERY: &str = "c7389bd0e731f80f49593e5ee53835475f4e28594dd6bd83eb229bab753498cd";
const BILLING_QUERY: &str = "c83b78a614689c38ebee981f9b39a8b377716db85c1fd7dbab604adc02d3313d";

pub fn normalize_cookie(raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ApiError::BadRequest("Cookie 不能为空".into()));
    }
    if raw.starts_with('[') || raw.starts_with('{') {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|_| ApiError::BadRequest("Cookie JSON 格式无效".into()))?;
        let items = value
            .as_array()
            .cloned()
            .or_else(|| value.get("cookies").and_then(|v| v.as_array()).cloned())
            .ok_or_else(|| ApiError::BadRequest("Cookie JSON 中缺少 cookies 数组".into()))?;
        let pairs: Vec<String> = items
            .iter()
            .filter_map(|item| {
                Some(format!(
                    "{}={}",
                    item.get("name")?.as_str()?,
                    item.get("value")?.as_str()?
                ))
            })
            .collect();
        return validate_pairs(pairs);
    }
    if raw.lines().any(|line| line.split('\t').count() >= 7) {
        let pairs = raw
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let columns: Vec<&str> = line.split('\t').collect();
                (columns.len() >= 7).then(|| format!("{}={}", columns[5], columns[6]))
            })
            .collect();
        return validate_pairs(pairs);
    }
    if raw.chars().any(|c| c == '\r' || c == '\n') {
        return Err(ApiError::BadRequest("Cookie 请求头不能包含换行符".into()));
    }
    let plain = raw.strip_prefix("Cookie:").unwrap_or(raw).trim();
    validate_pairs(
        plain
            .split(';')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn validate_pairs(pairs: Vec<String>) -> Result<String, ApiError> {
    if pairs.is_empty()
        || pairs
            .iter()
            .any(|p| !p.contains('=') || p.chars().any(char::is_control))
    {
        return Err(ApiError::BadRequest("Cookie 格式无效".into()));
    }
    Ok(pairs.join("; "))
}

async fn get(client: &Client, url: &str, cookie: &str) -> Result<reqwest::Response, ApiError> {
    client
        .get(url)
        .header(header::COOKIE, cookie)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "*/*")
        .send()
        .await
        .map_err(|e| ApiError::BadRequest(format!("连接 OpenCode 失败: {e}")))
}

pub async fn discover(
    client: &Client,
    cookie: &str,
) -> Result<(String, String, Option<String>), ApiError> {
    let response = get(client, &format!("{ORIGIN}/auth"), cookie).await?;
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let workspace = Regex::new(r"/workspace/(wrk_[A-Za-z0-9_-]+)")
        .unwrap()
        .captures(location)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| ApiError::BadRequest("Cookie 无效或已过期，未找到 workspace".into()))?;
    let keys = get(
        client,
        &format!("{ORIGIN}/workspace/{workspace}/keys"),
        cookie,
    )
    .await?;
    if !keys.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "读取账号密钥失败: HTTP {}",
            keys.status()
        )));
    }
    let body = keys
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let api_key = Regex::new(r"sk-[A-Za-z0-9_-]{20,}")
        .unwrap()
        .find(&body)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| ApiError::BadRequest("账号中未找到可用 API Key".into()))?;
    let email = extract_email(&body);
    Ok((workspace, api_key, email))
}

fn extract_email(body: &str) -> Option<String> {
    let decoded = body.replace("&quot;", "\"").replace("\\\"", "\"");
    let labelled = Regex::new(r#"(?i)["']email["']\s*:\s*["']([^"'<>\s]+@[^"'<>\s]+)["']"#).ok()?;
    if let Some(value) = labelled.captures(&decoded).and_then(|c| c.get(1)) {
        return Some(value.as_str().to_lowercase());
    }
    Regex::new(r#"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b"#)
        .ok()?
        .find(&decoded)
        .map(|m| m.as_str().to_lowercase())
}

async fn query(
    client: &Client,
    cookie: &str,
    workspace: &str,
    id: &str,
) -> Result<String, ApiError> {
    let args = format!(
        r#"{{"t":{{"t":9,"i":0,"l":1,"a":[{{"t":1,"s":"{workspace}"}}],"o":0}},"f":31,"m":[]}}"#
    );
    let response = client
        .get(format!("{ORIGIN}/_server"))
        .query(&[("id", id), ("args", args.as_str())])
        .header(header::COOKIE, cookie)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "*/*")
        .header(
            header::REFERER,
            format!("{ORIGIN}/workspace/{workspace}/usage"),
        )
        .header("x-server-id", id)
        .header("x-server-instance", "server-fn:0")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ApiError::BadRequest(format!("查询额度失败: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "查询额度失败: HTTP {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

fn number(body: &str, field: &str) -> Option<f64> {
    Regex::new(&format!(
        r#"["']?{}["']?\s*:\s*(?:"([0-9.]+)"|(-?[0-9.]+))"#,
        regex::escape(field)
    ))
    .ok()?
    .captures(body)
    .and_then(|c| c.get(1).or_else(|| c.get(2)))
    .and_then(|m| m.as_str().parse().ok())
}
fn string(body: &str, field: &str) -> Option<String> {
    Regex::new(&format!(
        r#"["']?{}["']?\s*:\s*"([^"]*)""#,
        regex::escape(field)
    ))
    .ok()?
    .captures(body)
    .and_then(|c| c.get(1))
    .map(|m| m.as_str().to_string())
}
fn window(body: &str, name: &str) -> Option<UsageWindow> {
    let start = body
        .find(&format!(r#""{name}""#))
        .or_else(|| body.find(name))?;
    let initial = &body[start..body.len().min(start + 200)];
    let reference = Regex::new(r"\$R\[(\d+)\]")
        .ok()?
        .captures(initial)
        .and_then(|c| c.get(1))
        .map(|m| format!("$R[{}]=", m.as_str()));
    let object_start = reference
        .as_ref()
        .and_then(|needle| body.find(needle))
        .unwrap_or(start);
    let tail = &body[object_start..body.len().min(object_start + 900)];
    let used = number(tail, "usagePercent")?;
    Some(UsageWindow {
        usage_percent: used,
        remaining_percent: (100.0 - used).max(0.0),
        reset_in_sec: number(tail, "resetInSec").unwrap_or(0.0) as i64,
        status: string(tail, "status").unwrap_or_else(|| "unknown".into()),
    })
}

pub async fn usage(
    client: &Client,
    cookie: &str,
    workspace: &str,
) -> Result<AccountUsage, ApiError> {
    let (subscription, billing) = tokio::try_join!(
        query(client, cookie, workspace, SUBSCRIPTION_QUERY),
        query(client, cookie, workspace, BILLING_QUERY)
    )?;
    let plan_name = plan_name(&subscription, &billing);
    Ok(build(plan_name, &subscription, &billing))
}

fn plan_name(subscription: &str, billing: &str) -> String {
    // OpenCode currently returns the Go subscription id from billing.get.
    // Keep checking lite.subscription.get as well for older response shapes.
    if has_go_subscription(subscription, billing) {
        "OpenCode Go".to_string()
    } else {
        string(billing, "subscriptionPlan").unwrap_or_else(|| "OpenCode Zen".to_string())
    }
}

fn has_go_subscription(subscription: &str, billing: &str) -> bool {
    [billing, subscription]
        .iter()
        .any(|body| string(body, "liteSubscriptionID").is_some_and(|id| !id.trim().is_empty()))
}

pub async fn invite_link(
    client: &Client,
    cookie: &str,
    workspace: &str,
) -> Result<String, ApiError> {
    let response = get(
        client,
        &format!("{ORIGIN}/workspace/{workspace}/go"),
        cookie,
    )
    .await?;
    if !response.status().is_success() {
        return Err(ApiError::BadRequest(format!(
            "读取邀请链接失败: HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    extract_invite_link(&body)
        .ok_or_else(|| ApiError::BadRequest("未找到邀请链接，请确认账号支持邀请功能".into()))
}

fn extract_invite_link(body: &str) -> Option<String> {
    let decoded = body.replace("&amp;", "&").replace("\\u0026", "&");
    Regex::new(r#"https://opencode\.ai/go\?ref=[A-Za-z0-9_-]+"#)
        .ok()?
        .find(&decoded)
        .map(|value| value.as_str().to_string())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ReferralServerFunctions {
    list: Option<String>,
    apply: Option<String>,
}

impl ReferralServerFunctions {
    fn record(&mut self, name: &str, id: &str) {
        match name {
            "go.referral.get" => self.list = Some(id.to_string()),
            "go.referral.reward.apply" => self.apply = Some(id.to_string()),
            _ => {}
        }
    }

    fn merge(&mut self, other: Self) {
        if other.list.is_some() {
            self.list = other.list;
        }
        if other.apply.is_some() {
            self.apply = other.apply;
        }
    }

    fn complete(&self) -> bool {
        self.list.is_some() && self.apply.is_some()
    }
}

pub async fn invite_rewards(
    client: &Client,
    cookie: &str,
    workspace: &str,
) -> Result<Vec<InviteReward>, ApiError> {
    let functions = referral_server_functions(client, cookie, workspace).await?;
    let list_id = functions.list.ok_or_else(|| {
        ApiError::Upstream("OpenCode 页面中未找到邀请奖励查询接口，请稍后重试".into())
    })?;
    query_invite_rewards(client, cookie, workspace, &list_id).await
}

pub async fn claim_invite_reward(
    client: &Client,
    cookie: &str,
    workspace: &str,
    reward_id: &str,
) -> Result<i64, ApiError> {
    let valid_id = Regex::new(r"^ref_[A-Za-z0-9_-]+$").expect("valid reward id regex");
    if !valid_id.is_match(reward_id) {
        return Err(ApiError::BadRequest("邀请奖励 ID 格式无效".into()));
    }

    let functions = referral_server_functions(client, cookie, workspace).await?;
    let list_id = functions.list.ok_or_else(|| {
        ApiError::Upstream("OpenCode 页面中未找到邀请奖励查询接口，请稍后重试".into())
    })?;
    let apply_id = functions.apply.ok_or_else(|| {
        ApiError::Upstream("OpenCode 页面中未找到邀请奖励领取接口，请稍后重试".into())
    })?;

    let rewards = query_invite_rewards(client, cookie, workspace, &list_id).await?;
    let reward = rewards
        .iter()
        .find(|reward| reward.id == reward_id)
        .ok_or_else(|| ApiError::NotFound("未找到该邀请奖励".into()))?;
    if !reward.claimable {
        return Err(ApiError::Conflict(format!(
            "该邀请奖励当前状态为 {}，不能领取",
            reward.status
        )));
    }

    apply_invite_reward(client, cookie, workspace, &apply_id, reward_id).await
}

async fn referral_server_functions(
    client: &Client,
    cookie: &str,
    workspace: &str,
) -> Result<ReferralServerFunctions, ApiError> {
    let go_url = format!("{ORIGIN}/workspace/{workspace}/go");
    let response = get(client, &go_url, cookie).await?;
    if !response.status().is_success() {
        return Err(ApiError::Upstream(format!(
            "读取邀请奖励页面失败: HTTP {}",
            response.status()
        )));
    }
    let html = response
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut functions = extract_referral_server_functions(&html);

    for asset_path in extract_javascript_assets(&html) {
        if functions.complete() {
            break;
        }
        let response = client
            .get(format!("{ORIGIN}{asset_path}"))
            .header(header::COOKIE, cookie)
            .header(header::USER_AGENT, USER_AGENT)
            .header(header::ACCEPT, "*/*")
            .header(header::REFERER, &go_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await;
        let Ok(response) = response else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bundle) = response.text().await else {
            continue;
        };
        functions.merge(extract_referral_server_functions(&bundle));
    }

    Ok(functions)
}

fn extract_javascript_assets(html: &str) -> Vec<String> {
    let pattern = Regex::new(
        r#"(?:href|src)\s*=\s*(?:\"(/_build/assets/[^\"]+\.js)\"|'(/_build/assets/[^']+\.js)')"#,
    )
    .expect("valid asset regex");
    let mut seen = HashSet::new();
    pattern
        .captures_iter(html)
        .filter_map(|capture| capture.get(1).or_else(|| capture.get(2)))
        .map(|value| value.as_str().to_string())
        .filter(|path| seen.insert(path.clone()))
        .take(64)
        .collect()
}

fn extract_referral_server_functions(bundle: &str) -> ReferralServerFunctions {
    let reference_pattern = Regex::new(
        r#"([A-Za-z_$][A-Za-z0-9_$]*)\s*=\s*(?:[A-Za-z_$][A-Za-z0-9_$]*\.)?createServerReference\(\s*[\"']([0-9a-fA-F]{64})[\"']"#,
    )
    .expect("valid server reference regex");
    let mut functions = ReferralServerFunctions::default();

    for capture in reference_pattern.captures_iter(bundle) {
        let Some(variable) = capture.get(1).map(|value| value.as_str()) else {
            continue;
        };
        let Some(id) = capture.get(2).map(|value| value.as_str()) else {
            continue;
        };
        let usage_pattern = Regex::new(&format!(
            r#"[A-Za-z_$][A-Za-z0-9_$.]*\s*\(\s*{}\s*,\s*[\"'](go\.referral\.(?:get|reward\.apply))[\"']"#,
            regex::escape(variable)
        ))
        .expect("valid server reference usage regex");
        if let Some(name) = usage_pattern
            .captures(bundle)
            .and_then(|usage| usage.get(1))
        {
            functions.record(name.as_str(), id);
        }
    }

    functions
}

async fn query_invite_rewards(
    client: &Client,
    cookie: &str,
    workspace: &str,
    server_id: &str,
) -> Result<Vec<InviteReward>, ApiError> {
    let args = referral_list_args(workspace);
    let referer = format!("{ORIGIN}/workspace/{workspace}/go");
    let response = client
        .get(format!("{ORIGIN}/_server"))
        .query(&[("id", server_id), ("args", args.as_str())])
        .header(header::COOKIE, cookie)
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "*/*")
        .header(header::REFERER, referer)
        .header("x-server-id", server_id)
        .header("x-server-instance", "server-fn:0")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ApiError::Upstream(format!("查询邀请奖励失败: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::Upstream(format!(
            "查询邀请奖励失败: HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rewards = parse_invite_rewards(&body);
    if rewards.is_empty() && body.contains("ref_") {
        return Err(ApiError::Upstream(
            "OpenCode 邀请奖励响应格式已变化，暂时无法解析".into(),
        ));
    }
    Ok(rewards)
}

async fn apply_invite_reward(
    client: &Client,
    cookie: &str,
    workspace: &str,
    server_id: &str,
    reward_id: &str,
) -> Result<i64, ApiError> {
    let referer = format!("{ORIGIN}/workspace/{workspace}/go");
    let response = client
        .post(format!("{ORIGIN}/_server"))
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::USER_AGENT, USER_AGENT)
        .header(header::ACCEPT, "*/*")
        .header(header::ORIGIN, ORIGIN)
        .header(header::REFERER, referer)
        .header("x-server-id", server_id)
        .header("x-server-instance", "server-fn:1")
        .header("x-single-flight", "true")
        .body(referral_apply_body(workspace, reward_id))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| ApiError::Upstream(format!("领取邀请奖励失败: {e}")))?;
    if !response.status().is_success() {
        return Err(ApiError::Upstream(format!(
            "领取邀请奖励失败: HTTP {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    number(&body, "amount")
        .map(|amount| amount as i64)
        .ok_or_else(|| ApiError::Upstream("奖励申请已发送，但 OpenCode 未返回奖励金额".into()))
}

fn referral_list_args(workspace: &str) -> String {
    serde_json::json!({
        "t": { "t": 9, "i": 0, "l": 1, "a": [{ "t": 1, "s": workspace }], "o": 0 },
        "f": 31,
        "m": []
    })
    .to_string()
}

fn referral_apply_body(workspace: &str, reward_id: &str) -> String {
    serde_json::json!({
        "t": {
            "t": 9,
            "i": 0,
            "l": 2,
            "a": [{ "t": 1, "s": workspace }, { "t": 1, "s": reward_id }],
            "o": 0
        },
        "f": 31,
        "m": []
    })
    .to_string()
}

fn parse_invite_rewards(body: &str) -> Vec<InviteReward> {
    let object_pattern = Regex::new(
        r#"(?s)\{[^{}]{0,4096}[\"']?id[\"']?\s*:\s*[\"']ref_[A-Za-z0-9_-]+[\"'][^{}]{0,4096}\}"#,
    )
    .expect("valid referral object regex");
    let mut seen = HashSet::new();
    let mut rewards = Vec::new();

    for value in object_pattern.find_iter(body) {
        let object = value.as_str();
        let Some(id) = object_string(object, "id") else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let status = object_string(object, "status").unwrap_or_else(|| "unknown".into());
        let created_at = [
            "createdAt",
            "timeCreated",
            "created_at",
            "time_created",
            "date",
        ]
        .iter()
        .find_map(|field| object_scalar(object, field));
        rewards.push(InviteReward {
            id,
            source: object_string(object, "source").unwrap_or_else(|| "unknown".into()),
            claimable: is_claimable_reward_status(&status),
            status,
            email: object_string(object, "email").unwrap_or_default(),
            amount_cents: object_integer(object, "amount").unwrap_or(0),
            created_at,
        });
    }

    rewards
}

fn is_claimable_reward_status(status: &str) -> bool {
    status.eq_ignore_ascii_case("available") || status.eq_ignore_ascii_case("pending")
}

fn object_string(body: &str, field: &str) -> Option<String> {
    let escaped = regex::escape(field);
    Regex::new(&format!(
        r#"(?:\"{escaped}\"|'{escaped}'|{escaped})\s*:\s*(?:\"([^\"]*)\"|'([^']*)')"#
    ))
    .ok()?
    .captures(body)
    .and_then(|capture| capture.get(1).or_else(|| capture.get(2)))
    .map(|value| value.as_str().to_string())
}

fn object_integer(body: &str, field: &str) -> Option<i64> {
    let escaped = regex::escape(field);
    Regex::new(&format!(
        r#"(?:\"{escaped}\"|'{escaped}'|{escaped})\s*:\s*(?:\"(-?\d+)\"|'(-?\d+)'|(-?\d+))"#
    ))
    .ok()?
    .captures(body)
    .and_then(|capture| {
        capture
            .get(1)
            .or_else(|| capture.get(2))
            .or_else(|| capture.get(3))
    })
    .and_then(|value| value.as_str().parse().ok())
}

fn object_scalar(body: &str, field: &str) -> Option<String> {
    object_string(body, field)
        .or_else(|| object_integer(body, field).map(|value| value.to_string()))
}

fn build(plan_name: String, subscription: &str, billing: &str) -> AccountUsage {
    AccountUsage {
        plan_name,
        plan_status: string(subscription, "status")
            .or_else(|| string(billing, "subscriptionStatus"))
            .unwrap_or_else(|| "unknown".into()),
        region: string(subscription, "region"),
        balance_microcents: number(billing, "balance").map(|v| v as i64),
        monthly_limit_microcents: number(billing, "monthlyLimit").map(|v| v as i64),
        monthly_usage_microcents: number(billing, "monthlyUsage").map(|v| v as i64),
        rolling: window(subscription, "rollingUsage"),
        weekly: window(subscription, "weeklyUsage"),
        monthly: window(subscription, "monthlyUsage"),
        fetched_at: now_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_plain_cookie() {
        assert_eq!(normalize_cookie("a=1; b=2").unwrap(), "a=1; b=2");
    }
    #[test]
    fn parses_json_cookie() {
        assert_eq!(
            normalize_cookie(r#"[{"name":"a","value":"1"}]"#).unwrap(),
            "a=1"
        );
    }
    #[test]
    fn parses_usage_window() {
        let w = window(
            r#""weeklyUsage":{"usagePercent":37.5,"resetInSec":60,"status":"ok"}"#,
            "weeklyUsage",
        )
        .unwrap();
        assert_eq!(w.remaining_percent, 62.5);
    }

    #[test]
    fn detects_go_from_quoted_lite_subscription_id() {
        assert_eq!(
            plan_name(r#"{"liteSubscriptionID":"sub_123"}"#, "{}"),
            "OpenCode Go"
        );
    }

    #[test]
    fn detects_go_from_unquoted_lite_subscription_id() {
        assert_eq!(
            plan_name(r#"{liteSubscriptionID:"sub_123"}"#, "{}"),
            "OpenCode Go"
        );
    }

    #[test]
    fn detects_go_from_billing_response() {
        assert_eq!(
            plan_name(
                r#"{region:"us",rollingUsage:null}"#,
                r#"{subscription:$R[1]={liteSubscriptionID:"sub_123"}}"#,
            ),
            "OpenCode Go"
        );
    }

    #[test]
    fn ignores_empty_go_subscription_id() {
        assert_eq!(
            plan_name("{}", r#"{liteSubscriptionID:""}"#),
            "OpenCode Zen"
        );
    }

    #[test]
    fn uses_billing_plan_without_go_subscription() {
        assert_eq!(
            plan_name("{}", r#"{"subscriptionPlan":"OpenCode Business"}"#),
            "OpenCode Business"
        );
    }
    #[test]
    fn extracts_email_from_serialized_session() {
        assert_eq!(
            extract_email(r#"{\"email\":\"User@Example.com\"}"#).as_deref(),
            Some("user@example.com")
        );
    }

    #[test]
    fn extracts_invite_link_from_rendered_page() {
        assert_eq!(
            extract_invite_link(
                r#"<script>const url=\"https://opencode.ai/go?ref=abc_123-XYZ\"</script>"#
            )
            .as_deref(),
            Some("https://opencode.ai/go?ref=abc_123-XYZ")
        );
    }

    #[test]
    fn extracts_referral_server_function_ids_from_bundle() {
        let list_id = "a".repeat(64);
        let apply_id = "b".repeat(64);
        let bundle = format!(
            r#"const listRef=createServerReference("{list_id}",()=>{{}});query(listRef,"go.referral.get");const applyRef=runtime.createServerReference('{apply_id}',()=>{{}});actions.run(applyRef,'go.referral.reward.apply');"#
        );

        assert_eq!(
            extract_referral_server_functions(&bundle),
            ReferralServerFunctions {
                list: Some(list_id),
                apply: Some(apply_id),
            }
        );
    }

    #[test]
    fn extracts_unique_javascript_assets() {
        assert_eq!(
            extract_javascript_assets(
                r#"<link href="/_build/assets/go-a.js"><script src='/_build/assets/shared-b.js'></script><script src="/_build/assets/go-a.js"></script>"#
            ),
            vec![
                "/_build/assets/go-a.js".to_string(),
                "/_build/assets/shared-b.js".to_string(),
            ]
        );
    }

    #[test]
    fn parses_referral_rewards_with_dates_and_claimability() {
        let body = r#"rewards:[{status:"available",email:"new@example.com",amount:500,id:"ref_available-1",source:"inviter"},{status:"pending",email:"friend@example.com",amount:500,id:"ref_pending-1",source:"inviter",createdAt:"2026-08-12T10:00:00.000Z"},{id:'ref_used_2',source:'invitee',status:'applied',email:'owner@example.com',amount:'500',timeCreated:1786500000000}]"#;

        assert_eq!(
            parse_invite_rewards(body),
            vec![
                InviteReward {
                    id: "ref_available-1".into(),
                    source: "inviter".into(),
                    status: "available".into(),
                    email: "new@example.com".into(),
                    amount_cents: 500,
                    created_at: None,
                    claimable: true,
                },
                InviteReward {
                    id: "ref_pending-1".into(),
                    source: "inviter".into(),
                    status: "pending".into(),
                    email: "friend@example.com".into(),
                    amount_cents: 500,
                    created_at: Some("2026-08-12T10:00:00.000Z".into()),
                    claimable: true,
                },
                InviteReward {
                    id: "ref_used_2".into(),
                    source: "invitee".into(),
                    status: "applied".into(),
                    email: "owner@example.com".into(),
                    amount_cents: 500,
                    created_at: Some("1786500000000".into()),
                    claimable: false,
                },
            ]
        );
    }

    #[test]
    fn builds_referral_rpc_arguments() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&referral_list_args("wrk_test")).unwrap(),
            serde_json::json!({
                "t": { "t": 9, "i": 0, "l": 1, "a": [{ "t": 1, "s": "wrk_test" }], "o": 0 },
                "f": 31,
                "m": []
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&referral_apply_body("wrk_test", "ref_test"))
                .unwrap(),
            serde_json::json!({
                "t": {
                    "t": 9,
                    "i": 0,
                    "l": 2,
                    "a": [{ "t": 1, "s": "wrk_test" }, { "t": 1, "s": "ref_test" }],
                    "o": 0
                },
                "f": 31,
                "m": []
            })
        );
    }
}
