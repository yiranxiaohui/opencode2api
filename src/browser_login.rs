use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tokio_tungstenite::connect_async;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::ApiError;
use crate::middleware::ManagementAuth;
use crate::models::{AccountType, CookieImportInput, KeyRecord, now_secs};
use crate::proxy_bridge::ProxyBridge;
use crate::state::AppState;

const LOGIN_URL: &str = "https://opencode.ai/auth";
const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const PROCESS_START_TIMEOUT: Duration = Duration::from_secs(15);
const ALIPAY_SELECTION_INTERVAL: Duration = Duration::from_millis(500);
const ALIPAY_CLICK_EXPRESSION: &str = r#"
(() => {
  const normalize = (value) => (value || '').replace(/\s+/g, ' ').trim().toLowerCase();
  const visible = (element) => {
    const style = window.getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return style.display !== 'none'
      && style.visibility !== 'hidden'
      && Number(style.opacity || 1) > 0
      && rect.width > 0
      && rect.height > 0;
  };
  const elements = Array.from(document.querySelectorAll('body *'));
  const heading = elements.find((element) => {
    if (!visible(element)) return false;
    const text = normalize(element.innerText);
    return text === 'select payment method' || text === '选择支付方式';
  });
  if (!heading) return false;

  let container = heading.parentElement;
  while (container && container !== document.body) {
    const alipay = Array.from(container.querySelectorAll('*')).find((element) => {
      if (!visible(element)) return false;
      const text = normalize(element.innerText);
      return text === 'alipay' || text === '支付宝';
    });
    if (alipay) {
      if (alipay.closest('button:disabled, [aria-disabled="true"]')) return false;
      const control = alipay.closest('button, a, [role="button"], [tabindex]');
      const target = control && container.contains(control) ? control : alipay;
      target.click();
      return true;
    }
    container = container.parentElement;
  }
  return false;
})()
"#;

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserLoginInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub invite_link: Option<String>,
    #[serde(default)]
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub account_type: AccountType,
}

#[derive(Debug, Serialize)]
pub struct BrowserLoginStarted {
    pub id: String,
    pub expires_at: i64,
}

#[derive(Debug, Serialize)]
pub struct BrowserLoginStatus {
    pub ready: bool,
}

#[derive(Clone, Default)]
pub struct BrowserLoginManager {
    session: Arc<Mutex<Option<BrowserSession>>>,
}

struct BrowserSession {
    id: String,
    expires_at: i64,
    input: Option<BrowserLoginInput>,
    proxy_revision: Option<String>,
    vnc_addr: SocketAddr,
    cdp_port: u16,
    profile_dir: PathBuf,
    chromium: Child,
    x11vnc: Child,
    xvfb: Child,
    alipay_selector: Option<JoinHandle<()>>,
    _proxy_bridge: Option<ProxyBridge>,
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        let _ = self.chromium.start_kill();
        let _ = self.x11vnc.start_kill();
        let _ = self.xvfb.start_kill();
        if let Some(selector) = self.alipay_selector.take() {
            selector.abort();
        }
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

impl BrowserSession {
    async fn stop(mut self) {
        let _ = self.chromium.kill().await;
        let _ = self.x11vnc.kill().await;
        let _ = self.xvfb.kill().await;
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }

    fn is_running(&mut self) -> Result<bool, ApiError> {
        let chromium = self.chromium.try_wait()?.is_none();
        let x11vnc = self.x11vnc.try_wait()?.is_none();
        let xvfb = self.xvfb.try_wait()?.is_none();
        Ok(chromium && x11vnc && xvfb && now_secs() < self.expires_at)
    }
}

impl BrowserLoginManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        &self,
        input: BrowserLoginInput,
        proxy_url: Option<Zeroizing<String>>,
        proxy_revision: Option<String>,
    ) -> Result<BrowserLoginStarted, ApiError> {
        let target_url = browser_login_url(input.invite_link.as_deref())?;
        self.start_session(
            Some(input),
            &target_url,
            None,
            proxy_url,
            proxy_revision,
            false,
        )
        .await
    }

    pub async fn start_account_page(
        &self,
        target_url: String,
        cookie: Zeroizing<String>,
        proxy_url: Option<Zeroizing<String>>,
    ) -> Result<BrowserLoginStarted, ApiError> {
        self.start_session(None, &target_url, Some(&cookie), proxy_url, None, true)
            .await
    }

    async fn start_session(
        &self,
        input: Option<BrowserLoginInput>,
        target_url: &str,
        cookie: Option<&str>,
        proxy_url: Option<Zeroizing<String>>,
        proxy_revision: Option<String>,
        auto_select_alipay: bool,
    ) -> Result<BrowserLoginStarted, ApiError> {
        let mut slot = self.session.lock().await;
        if let Some(current) = slot.as_mut()
            && current.is_running()?
        {
            return Err(ApiError::Conflict(
                "已有远程浏览器会话正在进行，请先完成或关闭该窗口".into(),
            ));
        }
        if let Some(stale) = slot.take() {
            stale.stop().await;
        }

        let session = launch_session(
            input,
            target_url,
            cookie,
            proxy_url.as_ref().map(|url| url.as_str()),
            proxy_revision,
            auto_select_alipay,
        )
        .await?;
        let started = BrowserLoginStarted {
            id: session.id.clone(),
            expires_at: session.expires_at,
        };
        *slot = Some(session);
        drop(slot);

        let manager = self.clone();
        let id = started.id.clone();
        tokio::spawn(async move {
            sleep(SESSION_TTL).await;
            let _ = manager.stop(&id).await;
        });
        Ok(started)
    }

    async fn with_session<T>(
        &self,
        id: &str,
        f: impl FnOnce(&mut BrowserSession) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let mut slot = self.session.lock().await;
        let session = slot
            .as_mut()
            .filter(|session| session.id == id)
            .ok_or_else(|| ApiError::NotFound("远程浏览器会话不存在或已结束".into()))?;
        if !session.is_running()? {
            let stale = slot.take().unwrap();
            drop(slot);
            stale.stop().await;
            return Err(ApiError::NotFound("远程浏览器会话已结束".into()));
        }
        f(session)
    }

    pub async fn vnc_addr(&self, id: &str) -> Result<SocketAddr, ApiError> {
        self.with_session(id, |session| Ok(session.vnc_addr)).await
    }

    pub async fn capture(
        &self,
        id: &str,
    ) -> Result<(String, BrowserLoginInput, Option<String>), ApiError> {
        let (port, input, proxy_revision) = self
            .with_session(id, |session| {
                Ok((
                    session.cdp_port,
                    session.input.clone().ok_or_else(|| {
                        ApiError::BadRequest("该远程浏览器会话不支持导入 Cookie".into())
                    })?,
                    session.proxy_revision.clone(),
                ))
            })
            .await?;
        let cookie = timeout(Duration::from_secs(5), cdp_cookie_header(port))
            .await
            .map_err(|_| ApiError::ServiceUnavailable("读取 Chromium Cookie 超时".into()))??;
        Ok((cookie, input, proxy_revision))
    }

    pub async fn ready(&self, id: &str) -> Result<bool, ApiError> {
        let port = self
            .with_session(id, |session| Ok(session.cdp_port))
            .await?;
        browser_is_ready(port).await
    }

    pub async fn stop(&self, id: &str) -> Result<(), ApiError> {
        let mut slot = self.session.lock().await;
        let matches = slot.as_ref().is_some_and(|session| session.id == id);
        if !matches {
            return Ok(());
        }
        let session = slot.take().unwrap();
        drop(slot);
        session.stop().await;
        Ok(())
    }
}

pub async fn start(
    State(st): State<AppState>,
    _: ManagementAuth,
    Json(input): Json<BrowserLoginInput>,
) -> Result<Json<BrowserLoginStarted>, ApiError> {
    let (proxy_url, proxy_revision) = if let Some(proxy_id) = input.proxy_id.as_deref() {
        let proxy = st
            .db
            .get_proxy(proxy_id)?
            .ok_or_else(|| ApiError::BadRequest("proxy not found".into()))?;
        (
            Some(st.decrypt_secret(&proxy.url_enc).await?),
            Some(proxy.url_enc),
        )
    } else {
        (None, None)
    };
    Ok(Json(
        st.browser_login
            .start(input, proxy_url, proxy_revision)
            .await?,
    ))
}

pub async fn capture(
    State(st): State<AppState>,
    _: ManagementAuth,
    AxumPath(id): AxumPath<String>,
) -> Result<(axum::http::StatusCode, Json<KeyRecord>), ApiError> {
    let (cookie, input, proxy_revision) = st.browser_login.capture(&id).await?;
    if let (Some(proxy_id), Some(expected_revision)) = (input.proxy_id.as_deref(), proxy_revision) {
        let proxy = st
            .db
            .get_proxy(proxy_id)?
            .ok_or_else(|| ApiError::BadRequest("登录期间绑定代理已被删除，请重新登录".into()))?;
        if proxy.url_enc != expected_revision {
            return Err(ApiError::BadRequest(
                "登录期间绑定代理已被修改，请重新登录以保持出口 IP 一致".into(),
            ));
        }
    }
    let record = crate::routes::keys::import_cookie_record(
        &st,
        CookieImportInput {
            cookie,
            name: input.name,
            proxy_id: input.proxy_id,
            account_type: input.account_type,
        },
    )
    .await?;
    if let Err(error) = st.browser_login.stop(&id).await {
        tracing::warn!("failed to stop completed browser login session: {error}");
    }
    Ok((axum::http::StatusCode::CREATED, Json(record)))
}

pub async fn start_go(
    State(st): State<AppState>,
    _: ManagementAuth,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<BrowserLoginStarted>, ApiError> {
    let account = st
        .db
        .get_key(&id)?
        .ok_or_else(|| ApiError::NotFound("account not found".into()))?;
    let cookie_enc = account.cookie_enc.as_deref().ok_or_else(|| {
        ApiError::BadRequest("该账号没有 Cookie，请先通过网页登录导入账号".into())
    })?;
    let workspace_id = account
        .workspace_id
        .as_deref()
        .ok_or_else(|| ApiError::BadRequest("该账号缺少 workspace，无法打开 Go 订阅页面".into()))?;
    let cookie = st.decrypt_secret(cookie_enc).await?;
    let proxy_url = if let Some(proxy_id) = account.proxy_id.as_deref() {
        let proxy = st
            .db
            .get_proxy(proxy_id)?
            .ok_or_else(|| ApiError::BadRequest("账号绑定代理不存在，请先编辑账号".into()))?;
        Some(st.decrypt_secret(&proxy.url_enc).await?)
    } else {
        None
    };
    let target_url = go_subscription_url(workspace_id)?;
    Ok(Json(
        st.browser_login
            .start_account_page(target_url, cookie, proxy_url)
            .await?,
    ))
}

pub async fn status(
    State(st): State<AppState>,
    _: ManagementAuth,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<BrowserLoginStatus>, ApiError> {
    Ok(Json(BrowserLoginStatus {
        ready: st.browser_login.ready(&id).await?,
    }))
}

pub async fn stop(
    State(st): State<AppState>,
    _: ManagementAuth,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<crate::models::OkResponse>, ApiError> {
    st.browser_login.stop(&id).await?;
    Ok(Json(crate::models::OkResponse { ok: true }))
}

pub async fn vnc(
    State(st): State<AppState>,
    _: ManagementAuth,
    AxumPath(id): AxumPath<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let addr = st.browser_login.vnc_addr(&id).await?;
    Ok(ws
        .on_upgrade(move |socket| async move {
            if let Err(error) = proxy_vnc(socket, addr).await {
                tracing::warn!("remote browser VNC connection ended: {error}");
            }
        })
        .into_response())
}

async fn proxy_vnc(socket: WebSocket, addr: SocketAddr) -> Result<(), ApiError> {
    let tcp = TcpStream::connect(addr)
        .await
        .map_err(|error| ApiError::ServiceUnavailable(format!("连接远程浏览器失败: {error}")))?;
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    let (mut ws_write, mut ws_read) = socket.split();
    let mut buffer = vec![0_u8; 64 * 1024];

    loop {
        tokio::select! {
            incoming = ws_read.next() => match incoming {
                Some(Ok(Message::Binary(data))) => tcp_write.write_all(&data).await?,
                Some(Ok(Message::Ping(data))) => ws_write.send(Message::Pong(data)).await
                    .map_err(|error| ApiError::Internal(format!("VNC WebSocket: {error}")))?,
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Text(_))) | Some(Ok(Message::Pong(_))) => {}
                Some(Err(error)) => return Err(ApiError::Internal(format!("VNC WebSocket: {error}"))),
            },
            read = tcp_read.read(&mut buffer) => {
                let read = read?;
                if read == 0 {
                    break;
                }
                ws_write
                    .send(Message::Binary(buffer[..read].to_vec().into()))
                    .await
                    .map_err(|error| ApiError::Internal(format!("VNC WebSocket: {error}")))?;
            }
        }
    }
    Ok(())
}

async fn launch_session(
    input: Option<BrowserLoginInput>,
    target_url: &str,
    cookie: Option<&str>,
    proxy_url: Option<&str>,
    proxy_revision: Option<String>,
    auto_select_alipay: bool,
) -> Result<BrowserSession, ApiError> {
    let chromium_bin = chromium_binary()?;
    let proxy_bridge = match proxy_url {
        Some(url) => Some(ProxyBridge::start(url).await?),
        None => None,
    };
    let display = available_display()?;
    let vnc_port = available_port().await?;
    let mut cdp_port = available_port().await?;
    while cdp_port == vnc_port {
        cdp_port = available_port().await?;
    }
    let id = Uuid::new_v4().to_string();
    let profile_dir = std::env::temp_dir().join(format!("opencode2api-browser-{id}"));
    std::fs::create_dir_all(&profile_dir)?;

    let mut xvfb_command = Command::new(env_binary("OPENCODE2API_XVFB_BIN", "Xvfb"));
    xvfb_command
        .args([
            format!(":{display}"),
            "-screen".into(),
            "0".into(),
            "1280x800x24".into(),
            "-ac".into(),
            "-nolisten".into(),
            "tcp".into(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut xvfb = xvfb_command
        .spawn()
        .map_err(|error| dependency_error("Xvfb", "OPENCODE2API_XVFB_BIN", error))?;
    if let Err(error) = wait_for_display(display, &mut xvfb).await {
        let _ = xvfb.kill().await;
        let _ = std::fs::remove_dir_all(&profile_dir);
        return Err(error);
    }

    let mut vnc_command = Command::new(env_binary("OPENCODE2API_X11VNC_BIN", "x11vnc"));
    vnc_command
        .args([
            "-display".into(),
            format!(":{display}"),
            "-rfbport".into(),
            vnc_port.to_string(),
            "-localhost".into(),
            "-forever".into(),
            "-shared".into(),
            "-nopw".into(),
            "-noxdamage".into(),
            "-quiet".into(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut x11vnc = match vnc_command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = xvfb.kill().await;
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(dependency_error("x11vnc", "OPENCODE2API_X11VNC_BIN", error));
        }
    };
    if let Err(error) = wait_for_tcp(vnc_port, &mut x11vnc, "x11vnc").await {
        let _ = x11vnc.kill().await;
        let _ = xvfb.kill().await;
        let _ = std::fs::remove_dir_all(&profile_dir);
        return Err(error);
    }

    let mut chromium_command = Command::new(chromium_bin);
    chromium_command
        .env("DISPLAY", format!(":{display}"))
        .args([
            "--ozone-platform=x11".into(),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-dev-shm-usage".into(),
            "--disable-background-networking".into(),
            "--disable-component-update".into(),
            "--disable-sync".into(),
            "--disable-features=Translate".into(),
            "--window-size=1280,800".into(),
            "--start-maximized".into(),
            format!("--user-data-dir={}", profile_dir.display()),
            "--remote-debugging-address=127.0.0.1".into(),
            format!("--remote-debugging-port={cdp_port}"),
        ])
        .arg(if cookie.is_some() {
            "about:blank"
        } else {
            target_url
        })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(bridge) = proxy_bridge.as_ref() {
        chromium_command
            .arg(format!("--proxy-server=http://{}", bridge.addr()))
            .arg("--disable-quic")
            .arg("--force-webrtc-ip-handling-policy=disable_non_proxied_udp");
    }
    if env_flag("OPENCODE2API_CHROMIUM_NO_SANDBOX") {
        chromium_command.arg("--no-sandbox");
    }
    let mut chromium = match chromium_command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = x11vnc.kill().await;
            let _ = xvfb.kill().await;
            let _ = std::fs::remove_dir_all(&profile_dir);
            return Err(ApiError::ServiceUnavailable(format!(
                "无法启动 Chromium: {error}"
            )));
        }
    };
    if let Err(error) = wait_for_cdp(cdp_port, &mut chromium).await {
        let _ = chromium.kill().await;
        let _ = x11vnc.kill().await;
        let _ = xvfb.kill().await;
        let _ = std::fs::remove_dir_all(&profile_dir);
        return Err(error);
    }
    let account_page = if let Some(cookie) = cookie {
        timeout(
            Duration::from_secs(5),
            cdp_open_authenticated_page(cdp_port, cookie, target_url),
        )
        .await
        .map_err(|_| ApiError::ServiceUnavailable("初始化账号浏览器超时".into()))?
    } else {
        Ok(())
    };
    if let Err(error) = account_page {
        let _ = chromium.kill().await;
        let _ = x11vnc.kill().await;
        let _ = xvfb.kill().await;
        let _ = std::fs::remove_dir_all(&profile_dir);
        return Err(error);
    }

    let alipay_selector = auto_select_alipay.then(|| {
        tokio::spawn(async move {
            watch_for_alipay_payment_method(cdp_port).await;
        })
    });

    Ok(BrowserSession {
        id,
        expires_at: now_secs() + SESSION_TTL.as_secs() as i64,
        input,
        proxy_revision,
        vnc_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), vnc_port),
        cdp_port,
        profile_dir,
        chromium,
        x11vnc,
        xvfb,
        alipay_selector,
        _proxy_bridge: proxy_bridge,
    })
}

fn env_binary(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn chromium_binary() -> Result<PathBuf, ApiError> {
    if let Ok(value) = std::env::var("OPENCODE2API_CHROMIUM_BIN") {
        return Ok(PathBuf::from(value));
    }
    [
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
    .ok_or_else(|| {
        ApiError::ServiceUnavailable(
            "未找到 Chromium；Docker 镜像已内置，直接运行二进制时请安装 Chromium 或设置 OPENCODE2API_CHROMIUM_BIN".into(),
        )
    })
}

fn available_display() -> Result<u16, ApiError> {
    let seed = Uuid::new_v4().as_u128() as u16;
    for offset in 0..100_u16 {
        let display = 100 + (seed.wrapping_add(offset) % 800);
        if !Path::new(&format!("/tmp/.X11-unix/X{display}")).exists() {
            return Ok(display);
        }
    }
    Err(ApiError::ServiceUnavailable(
        "没有可用的虚拟显示编号".into(),
    ))
}

async fn available_port() -> Result<u16, ApiError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_for_display(display: u16, child: &mut Child) -> Result<(), ApiError> {
    let socket = format!("/tmp/.X11-unix/X{display}");
    timeout(PROCESS_START_TIMEOUT, async {
        loop {
            if Path::new(&socket).exists() {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(ApiError::ServiceUnavailable(format!(
                    "Xvfb 启动失败: {status}"
                )));
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable("等待 Xvfb 启动超时".into()))?
}

async fn wait_for_tcp(port: u16, child: &mut Child, name: &str) -> Result<(), ApiError> {
    timeout(PROCESS_START_TIMEOUT, async {
        loop {
            if TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok()
            {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(ApiError::ServiceUnavailable(format!(
                    "{name} 启动失败: {status}"
                )));
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable(format!("等待 {name} 启动超时")))?
}

async fn wait_for_cdp(port: u16, child: &mut Child) -> Result<(), ApiError> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::new();
    timeout(PROCESS_START_TIMEOUT, async {
        loop {
            if client
                .get(&url)
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                return Err(ApiError::ServiceUnavailable(format!(
                    "Chromium 启动失败: {status}；容器中请启用 OPENCODE2API_CHROMIUM_NO_SANDBOX"
                )));
            }
            sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .map_err(|_| ApiError::ServiceUnavailable("等待 Chromium 启动超时".into()))?
}

fn dependency_error(name: &str, env_name: &str, error: std::io::Error) -> ApiError {
    ApiError::ServiceUnavailable(format!(
        "无法启动 {name}: {error}；请安装该程序或设置 {env_name}"
    ))
}

fn browser_login_url(invite_link: Option<&str>) -> Result<String, ApiError> {
    let Some(invite_link) = invite_link.map(str::trim).filter(|link| !link.is_empty()) else {
        return Ok(LOGIN_URL.to_string());
    };
    let url = reqwest::Url::parse(invite_link)
        .map_err(|_| ApiError::BadRequest("邀请链接格式无效".into()))?;
    let valid_origin = url.scheme() == "https"
        && url.host_str() == Some("opencode.ai")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !valid_origin || url.path() != "/go" || url.fragment().is_some() {
        return Err(ApiError::BadRequest(
            "仅支持 https://opencode.ai/go?ref=... 格式的邀请链接".into(),
        ));
    }

    let referrals = url
        .query_pairs()
        .filter(|(name, _)| name == "ref")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let [referral] = referrals.as_slice() else {
        return Err(ApiError::BadRequest("邀请链接缺少有效的 ref 参数".into()));
    };
    if referral.is_empty()
        || !referral
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(ApiError::BadRequest("邀请链接的 ref 参数格式无效".into()));
    }

    let mut target = reqwest::Url::parse("https://opencode.ai/go")
        .map_err(|error| ApiError::Internal(format!("创建邀请登录地址失败: {error}")))?;
    target.query_pairs_mut().append_pair("ref", referral);
    Ok(target.to_string())
}

fn go_subscription_url(workspace_id: &str) -> Result<String, ApiError> {
    if workspace_id.is_empty() || workspace_id.chars().any(char::is_control) {
        return Err(ApiError::BadRequest("workspace 格式无效".into()));
    }
    let mut url = reqwest::Url::parse("https://opencode.ai")
        .map_err(|error| ApiError::Internal(format!("创建 Go 订阅地址失败: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| ApiError::Internal("创建 Go 订阅地址失败".into()))?
        .extend(["workspace", workspace_id, "go"]);
    Ok(url.to_string())
}

async fn watch_for_alipay_payment_method(port: u16) {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::debug!("failed to initialize Alipay selector: {error}");
            return;
        }
    };
    let deadline = Instant::now() + SESSION_TTL;
    let mut last_error = None;

    while Instant::now() < deadline {
        match timeout(
            Duration::from_secs(3),
            try_select_alipay_payment_method(&client, port),
        )
        .await
        {
            Ok(Ok(true)) => {
                tracing::info!("automatically selected Alipay payment method");
                return;
            }
            Ok(Ok(false)) => {}
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some("Chromium DevTools request timed out".to_string()),
        }
        sleep(ALIPAY_SELECTION_INTERVAL).await;
    }

    if let Some(error) = last_error {
        tracing::debug!("Alipay selector ended without a click: {error}");
    }
}

async fn try_select_alipay_payment_method(
    client: &reqwest::Client,
    port: u16,
) -> Result<bool, ApiError> {
    let pages: Value = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|error| ApiError::ServiceUnavailable(format!("连接 Chromium 失败: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("解析 Chromium 页面失败: {error}")))?;
    let Some(websocket_url) = pages.as_array().and_then(|pages| {
        pages.iter().find_map(|page| {
            let is_opencode_page = page.get("type").and_then(Value::as_str) == Some("page")
                && page
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|url| url.starts_with("https://opencode.ai/workspace/"));
            is_opencode_page
                .then(|| page.get("webSocketDebuggerUrl").and_then(Value::as_str))
                .flatten()
        })
    }) else {
        return Ok(false);
    };
    let mut websocket_url = reqwest::Url::parse(websocket_url)
        .map_err(|error| ApiError::Internal(format!("Chromium DevTools 地址无效: {error}")))?;
    websocket_url
        .set_host(Some("127.0.0.1"))
        .map_err(|_| ApiError::Internal("Chromium DevTools 地址无效".into()))?;
    let (mut socket, _) = connect_async(websocket_url.as_str())
        .await
        .map_err(|error| {
            ApiError::ServiceUnavailable(format!("连接 Chromium DevTools 失败: {error}"))
        })?;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "id": 1,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": ALIPAY_CLICK_EXPRESSION,
                    "returnByValue": true,
                    "userGesture": true
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| ApiError::Internal(format!("选择支付宝失败: {error}")))?;

    while let Some(message) = socket.next().await {
        let message =
            message.map_err(|error| ApiError::Internal(format!("选择支付宝失败: {error}")))?;
        let Some(text) = (match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => Some(text.to_string()),
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                String::from_utf8(bytes.to_vec()).ok()
            }
            _ => None,
        }) else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        return alipay_click_result(&value);
    }
    Err(ApiError::ServiceUnavailable(
        "选择支付宝时 Chromium 连接已关闭".into(),
    ))
}

fn alipay_click_result(value: &Value) -> Result<bool, ApiError> {
    if value.get("error").is_some() || value.pointer("/result/exceptionDetails").is_some() {
        return Err(ApiError::Internal("执行支付宝自动选择失败".into()));
    }
    value
        .pointer("/result/result/value")
        .and_then(Value::as_bool)
        .ok_or_else(|| ApiError::Internal("支付宝自动选择响应格式无效".into()))
}

async fn browser_is_ready(port: u16) -> Result<bool, ApiError> {
    let pages: Value = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| ApiError::Internal(format!("创建 Chromium 客户端失败: {error}")))?
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|error| ApiError::ServiceUnavailable(format!("连接 Chromium 失败: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("解析 Chromium 页面状态失败: {error}")))?;
    Ok(pages.as_array().is_some_and(|pages| {
        pages.iter().any(|page| {
            page.get("type").and_then(Value::as_str) == Some("page")
                && page
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(is_authenticated_page)
        })
    }))
}

fn is_authenticated_page(url: &str) -> bool {
    url.starts_with("https://opencode.ai/workspace/")
}

async fn cdp_open_authenticated_page(
    port: u16,
    cookie_header: &str,
    target_url: &str,
) -> Result<(), ApiError> {
    let pages: Value = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| ApiError::Internal(format!("创建 Chromium 客户端失败: {error}")))?
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .map_err(|error| ApiError::ServiceUnavailable(format!("连接 Chromium 失败: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("解析 Chromium 页面失败: {error}")))?;
    let websocket_url = pages
        .as_array()
        .and_then(|pages| {
            pages.iter().find(|page| {
                page.get("type").and_then(Value::as_str) == Some("page")
                    && page.get("webSocketDebuggerUrl").is_some()
            })
        })
        .and_then(|page| page.get("webSocketDebuggerUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::Internal("Chromium 未返回页面 DevTools 地址".into()))?;
    let mut websocket_url = reqwest::Url::parse(websocket_url)
        .map_err(|error| ApiError::Internal(format!("Chromium DevTools 地址无效: {error}")))?;
    websocket_url
        .set_host(Some("127.0.0.1"))
        .map_err(|_| ApiError::Internal("Chromium DevTools 地址无效".into()))?;
    let (mut socket, _) = connect_async(websocket_url.as_str())
        .await
        .map_err(|error| {
            ApiError::ServiceUnavailable(format!("连接 Chromium DevTools 失败: {error}"))
        })?;

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({
                "id": 1,
                "method": "Network.setCookies",
                "params": {"cookies": cookie_params(cookie_header)?}
            })
            .to_string()
            .into(),
        ))
        .await
        .map_err(|error| ApiError::Internal(format!("写入 Chromium Cookie 失败: {error}")))?;
    wait_for_cdp_response(&mut socket, 1, "写入 Chromium Cookie").await?;

    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"id": 2, "method": "Page.navigate", "params": {"url": target_url}})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| ApiError::Internal(format!("打开 Go 订阅页面失败: {error}")))?;
    wait_for_cdp_response(&mut socket, 2, "打开 Go 订阅页面").await
}

async fn wait_for_cdp_response<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    id: i64,
    action: &str,
) -> Result<(), ApiError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = socket.next().await {
        let message =
            message.map_err(|error| ApiError::Internal(format!("{action}失败: {error}")))?;
        let Some(text) = (match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => Some(text.to_string()),
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                String::from_utf8(bytes.to_vec()).ok()
            }
            _ => None,
        }) else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(ApiError::Internal(format!("{action}失败: {error}")));
        }
        return Ok(());
    }
    Err(ApiError::ServiceUnavailable(format!("{action}连接已关闭")))
}

fn cookie_params(cookie_header: &str) -> Result<Vec<Value>, ApiError> {
    let mut cookies = Vec::new();
    for part in cookie_header.split(';') {
        let (name, value) = part.trim().split_once('=').ok_or_else(|| {
            ApiError::BadRequest("账号 Cookie 格式无效，请重新网页登录导入".into())
        })?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty()
            || name.chars().any(char::is_control)
            || value.chars().any(char::is_control)
        {
            return Err(ApiError::BadRequest(
                "账号 Cookie 格式无效，请重新网页登录导入".into(),
            ));
        }
        let cookie = if name.starts_with("__Host-") {
            json!({
                "name": name,
                "value": value,
                "url": "https://opencode.ai/",
                "path": "/",
                "secure": true,
                "httpOnly": true
            })
        } else {
            json!({
                "name": name,
                "value": value,
                "domain": ".opencode.ai",
                "path": "/",
                "secure": true,
                "httpOnly": true
            })
        };
        cookies.push(cookie);
    }
    if cookies.is_empty() {
        return Err(ApiError::BadRequest(
            "账号 Cookie 为空，请重新网页登录导入".into(),
        ));
    }
    Ok(cookies)
}

async fn cdp_cookie_header(port: u16) -> Result<String, ApiError> {
    let client = reqwest::Client::new();
    let version: Value = client
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
        .map_err(|error| ApiError::ServiceUnavailable(format!("连接 Chromium 失败: {error}")))?
        .json()
        .await
        .map_err(|error| ApiError::Internal(format!("解析 Chromium 状态失败: {error}")))?;
    let websocket_url = version
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::Internal("Chromium 未返回 DevTools 地址".into()))?;
    let mut websocket_url = reqwest::Url::parse(websocket_url)
        .map_err(|error| ApiError::Internal(format!("Chromium DevTools 地址无效: {error}")))?;
    websocket_url
        .set_host(Some("127.0.0.1"))
        .map_err(|_| ApiError::Internal("Chromium DevTools 地址无效".into()))?;
    let (mut socket, _) = connect_async(websocket_url.as_str())
        .await
        .map_err(|error| {
            ApiError::ServiceUnavailable(format!("连接 Chromium DevTools 失败: {error}"))
        })?;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"id": 1, "method": "Storage.getCookies"})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|error| ApiError::Internal(format!("读取 Chromium Cookie 失败: {error}")))?;

    while let Some(message) = socket.next().await {
        let message = message
            .map_err(|error| ApiError::Internal(format!("读取 Chromium Cookie 失败: {error}")))?;
        let Some(text) = (match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => Some(text.to_string()),
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                String::from_utf8(bytes.to_vec()).ok()
            }
            _ => None,
        }) else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(ApiError::Internal(format!(
                "Chromium Cookie 查询失败: {error}"
            )));
        }
        return cookie_header_from_cdp(&value);
    }
    Err(ApiError::ServiceUnavailable(
        "Chromium Cookie 查询连接已关闭".into(),
    ))
}

fn cookie_header_from_cdp(value: &Value) -> Result<String, ApiError> {
    let cookies = value
        .pointer("/result/cookies")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Internal("Chromium Cookie 响应格式无效".into()))?;
    let now = now_secs() as f64;
    let mut pairs: Vec<(&str, &str, usize)> = cookies
        .iter()
        .filter(|cookie| {
            let domain = cookie
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches('.');
            domain == "opencode.ai"
        })
        .filter(|cookie| cookie.get("path").and_then(Value::as_str).unwrap_or("/") == "/")
        .filter(|cookie| {
            cookie
                .get("expires")
                .and_then(Value::as_f64)
                .is_none_or(|expires| expires <= 0.0 || expires > now)
        })
        .filter_map(|cookie| {
            Some((
                cookie.get("name")?.as_str()?,
                cookie.get("value")?.as_str()?,
                cookie
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .len(),
            ))
        })
        .filter(|(name, value, _)| {
            !name.is_empty()
                && !name.chars().any(char::is_control)
                && !value.chars().any(char::is_control)
        })
        .collect();
    pairs.sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(right.0)));
    if pairs.is_empty() {
        return Err(ApiError::BadRequest(
            "未检测到 OpenCode Cookie，请先在窗口中完成登录".into(),
        ));
    }
    Ok(pairs
        .into_iter()
        .map(|(name, value, _)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_cookie_header_for_opencode_only_and_skips_expired_values() {
        let future = now_secs() + 3600;
        let value = json!({
            "result": {"cookies": [
                {"name": "root", "value": "a", "domain": ".opencode.ai", "path": "/", "expires": future},
                {"name": "session", "value": "b", "domain": "opencode.ai", "path": "/", "expires": -1},
                {"name": "auth-only", "value": "x", "domain": "opencode.ai", "path": "/auth", "expires": -1},
                {"name": "subdomain", "value": "y", "domain": "auth.opencode.ai", "path": "/", "expires": -1},
                {"name": "old", "value": "c", "domain": "opencode.ai", "path": "/", "expires": 1},
                {"name": "other", "value": "d", "domain": ".example.com", "path": "/", "expires": future}
            ]}
        });
        assert_eq!(cookie_header_from_cdp(&value).unwrap(), "root=a; session=b");
    }

    #[test]
    fn rejects_capture_before_opencode_sets_a_cookie() {
        let value = json!({"result": {"cookies": [
            {"name": "other", "value": "d", "domain": ".example.com", "path": "/", "expires": -1}
        ]}});
        assert!(matches!(
            cookie_header_from_cdp(&value),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn recognizes_workspace_as_authenticated_page() {
        assert!(is_authenticated_page(
            "https://opencode.ai/workspace/wrk_123/keys"
        ));
        assert!(!is_authenticated_page("https://opencode.ai/auth"));
        assert!(!is_authenticated_page(
            "https://example.com/workspace/wrk_123"
        ));
    }

    #[test]
    fn uses_login_page_without_an_invite_link() {
        assert_eq!(browser_login_url(None).unwrap(), LOGIN_URL);
        assert_eq!(browser_login_url(Some("  ")).unwrap(), LOGIN_URL);
    }

    #[test]
    fn accepts_and_normalizes_opencode_invite_links() {
        assert_eq!(
            browser_login_url(Some(
                "  https://opencode.ai/go?utm_source=test&ref=abc_123-XYZ  "
            ))
            .unwrap(),
            "https://opencode.ai/go?ref=abc_123-XYZ"
        );
    }

    #[test]
    fn rejects_untrusted_or_malformed_invite_links() {
        for link in [
            "https://example.com/go?ref=abc",
            "https://opencode.ai.example.com/go?ref=abc",
            "http://opencode.ai/go?ref=abc",
            "https://opencode.ai/auth?ref=abc",
            "https://opencode.ai/go",
            "https://opencode.ai/go?ref=abc%2Fdef",
            "https://opencode.ai/go?ref=one&ref=two",
        ] {
            assert!(
                matches!(browser_login_url(Some(link)), Err(ApiError::BadRequest(_))),
                "accepted invalid invite link: {link}"
            );
        }
    }

    #[test]
    fn builds_go_subscription_url_with_encoded_workspace() {
        assert_eq!(
            go_subscription_url("wrk_123/a").unwrap(),
            "https://opencode.ai/workspace/wrk_123%2Fa/go"
        );
    }

    #[test]
    fn parses_successful_alipay_click_result() {
        let clicked = json!({
            "id": 1,
            "result": {"result": {"type": "boolean", "value": true}}
        });
        let waiting = json!({
            "id": 1,
            "result": {"result": {"type": "boolean", "value": false}}
        });

        assert!(alipay_click_result(&clicked).unwrap());
        assert!(!alipay_click_result(&waiting).unwrap());
    }

    #[test]
    fn rejects_failed_or_malformed_alipay_click_result() {
        let failed = json!({"id": 1, "result": {"exceptionDetails": {}}});
        let malformed = json!({"id": 1, "result": {"result": {"type": "undefined"}}});

        assert!(matches!(
            alipay_click_result(&failed),
            Err(ApiError::Internal(_))
        ));
        assert!(matches!(
            alipay_click_result(&malformed),
            Err(ApiError::Internal(_))
        ));
    }

    #[test]
    fn converts_cookie_header_to_secure_cdp_cookie_params() {
        let cookies = cookie_params("session=abc==; __Host-auth=secret").unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0]["name"], "session");
        assert_eq!(cookies[0]["value"], "abc==");
        assert_eq!(cookies[0]["domain"], ".opencode.ai");
        assert_eq!(cookies[0]["secure"], true);
        assert_eq!(cookies[0]["httpOnly"], true);
        assert_eq!(cookies[1]["name"], "__Host-auth");
        assert_eq!(cookies[1]["url"], "https://opencode.ai/");
        assert!(cookies[1].get("domain").is_none());
    }

    #[test]
    fn rejects_invalid_cookie_header_for_account_browser() {
        assert!(matches!(
            cookie_params("missing-value-separator"),
            Err(ApiError::BadRequest(_))
        ));
    }
}
