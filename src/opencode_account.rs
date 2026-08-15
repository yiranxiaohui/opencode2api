use regex::Regex;
use reqwest::{Client, header};
use std::time::Duration;

use crate::error::ApiError;
use crate::models::{AccountUsage, UsageWindow, now_secs};

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
}
