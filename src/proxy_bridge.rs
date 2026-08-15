use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use percent_encoding::percent_decode_str;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, lookup_host};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use crate::error::ApiError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_HEADER_SIZE: usize = 64 * 1024;

pub struct ProxyBridge {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl ProxyBridge {
    pub async fn start(url: &str) -> Result<Self, ApiError> {
        let upstream = Arc::new(UpstreamProxy::parse(url)?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok((client, _)) => {
                            let upstream = upstream.clone();
                            connections.spawn(async move {
                                if let Err(error) = handle_client(client, &upstream).await {
                                    tracing::debug!("browser proxy bridge connection ended: {error}");
                                }
                            });
                        }
                        Err(error) => {
                            tracing::warn!("browser proxy bridge stopped accepting connections: {error}");
                            break;
                        }
                    },
                    result = connections.join_next(), if !connections.is_empty() => {
                        if let Some(Err(error)) = result {
                            tracing::debug!("browser proxy bridge task ended: {error}");
                        }
                    }
                }
            }
        });
        Ok(Self { addr, task })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for ProxyBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ProxyCredentials {
    username: Zeroizing<String>,
    password: Zeroizing<String>,
}

enum UpstreamProxy {
    Http {
        host: String,
        port: u16,
        tls: bool,
        credentials: Option<ProxyCredentials>,
    },
    Socks5 {
        host: String,
        port: u16,
        remote_dns: bool,
        credentials: Option<ProxyCredentials>,
    },
}

impl UpstreamProxy {
    fn parse(raw: &str) -> Result<Self, ApiError> {
        let url = reqwest::Url::parse(raw)
            .map_err(|error| ApiError::BadRequest(format!("invalid proxy URL: {error}")))?;
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| ApiError::BadRequest("proxy URL is missing a host".into()))?
            .to_string();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ApiError::BadRequest("proxy URL is missing a port".into()))?;
        let credentials = proxy_credentials(&url)?;
        match url.scheme() {
            "http" => Ok(Self::Http {
                host,
                port,
                tls: false,
                credentials,
            }),
            "https" => Ok(Self::Http {
                host,
                port,
                tls: true,
                credentials,
            }),
            "socks5" => Ok(Self::Socks5 {
                host,
                port,
                remote_dns: false,
                credentials,
            }),
            "socks5h" => Ok(Self::Socks5 {
                host,
                port,
                remote_dns: true,
                credentials,
            }),
            scheme => Err(ApiError::BadRequest(format!(
                "browser login does not support proxy scheme: {scheme}"
            ))),
        }
    }
}

fn proxy_credentials(url: &reqwest::Url) -> Result<Option<ProxyCredentials>, ApiError> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let username = percent_decode_str(url.username())
        .decode_utf8()
        .map_err(|_| ApiError::BadRequest("proxy username is not valid UTF-8".into()))?
        .into_owned();
    let password = percent_decode_str(url.password().unwrap_or_default())
        .decode_utf8()
        .map_err(|_| ApiError::BadRequest("proxy password is not valid UTF-8".into()))?
        .into_owned();
    if username.len() > u8::MAX as usize || password.len() > u8::MAX as usize {
        return Err(ApiError::BadRequest(
            "proxy username or password is too long".into(),
        ));
    }
    if username.chars().any(char::is_control) || password.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(
            "proxy username or password contains control characters".into(),
        ));
    }
    Ok(Some(ProxyCredentials {
        username: Zeroizing::new(username),
        password: Zeroizing::new(password),
    }))
}

trait AsyncIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncIo for T {}
type BoxIo = Box<dyn AsyncIo>;

async fn handle_client(mut client: TcpStream, proxy: &UpstreamProxy) -> io::Result<()> {
    let (head, trailing) = read_http_head(&mut client).await?;
    let request_line = std::str::from_utf8(&head)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid proxy request"))?
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty proxy request"))?;
    let mut fields = request_line.split_whitespace();
    let method = fields.next().unwrap_or_default();
    let authority = fields.next().unwrap_or_default();
    if method != "CONNECT" {
        client
            .write_all(
                b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            )
            .await?;
        return Ok(());
    }
    let (target_host, target_port) = parse_authority(authority, 443)?;
    let tunnel = timeout(
        CONNECT_TIMEOUT,
        open_tunnel(proxy, &target_host, target_port),
    )
    .await;
    let mut upstream = match tunnel {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
                .await;
            return Err(error);
        }
        Err(_) => {
            let _ = client
                .write_all(b"HTTP/1.1 504 Gateway Timeout\r\nConnection: close\r\n\r\n")
                .await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "upstream proxy tunnel timed out",
            ));
        }
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    if !trailing.is_empty() {
        upstream.write_all(&trailing).await?;
    }
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn open_tunnel(
    proxy: &UpstreamProxy,
    target_host: &str,
    target_port: u16,
) -> io::Result<BoxIo> {
    match proxy {
        UpstreamProxy::Http {
            host,
            port,
            tls,
            credentials,
        } => {
            open_http_tunnel(
                host,
                *port,
                *tls,
                credentials.as_ref(),
                target_host,
                target_port,
            )
            .await
        }
        UpstreamProxy::Socks5 {
            host,
            port,
            remote_dns,
            credentials,
        } => {
            open_socks5_tunnel(
                host,
                *port,
                *remote_dns,
                credentials.as_ref(),
                target_host,
                target_port,
            )
            .await
        }
    }
}

async fn open_http_tunnel(
    proxy_host: &str,
    proxy_port: u16,
    tls: bool,
    credentials: Option<&ProxyCredentials>,
    target_host: &str,
    target_port: u16,
) -> io::Result<BoxIo> {
    let tcp = TcpStream::connect((proxy_host, proxy_port)).await?;
    let mut stream: BoxIo = if tls {
        let config = ClientConfig::builder()
            .with_platform_verifier()
            .map_err(io::Error::other)?
            .with_no_client_auth();
        let server_name = ServerName::try_from(proxy_host.to_string())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid HTTPS proxy host"))?;
        let tls = TlsConnector::from(Arc::new(config))
            .connect(server_name, tcp)
            .await
            .map_err(io::Error::other)?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };

    let authority = format_authority(target_host, target_port);
    let mut request = Zeroizing::new(format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n"
    ));
    if let Some(credentials) = credentials {
        let raw = Zeroizing::new(format!(
            "{}:{}",
            credentials.username.as_str(),
            credentials.password.as_str()
        ));
        let encoded = Zeroizing::new(base64::prelude::BASE64_STANDARD.encode(raw.as_bytes()));
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(&encoded);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    let (response, trailing) = read_http_head(&mut stream).await?;
    if !http_response_is_success(&response) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "upstream HTTP proxy rejected CONNECT",
        ));
    }
    if !trailing.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "upstream HTTP proxy sent unexpected tunnel data",
        ));
    }
    Ok(stream)
}

async fn open_socks5_tunnel(
    proxy_host: &str,
    proxy_port: u16,
    remote_dns: bool,
    credentials: Option<&ProxyCredentials>,
    target_host: &str,
    target_port: u16,
) -> io::Result<BoxIo> {
    let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
    if credentials.is_some() {
        stream.write_all(&[5, 2, 0, 2]).await?;
    } else {
        stream.write_all(&[5, 1, 0]).await?;
    }
    let mut method = [0_u8; 2];
    stream.read_exact(&mut method).await?;
    if method[0] != 5 || method[1] == 0xff {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SOCKS5 proxy rejected authentication methods",
        ));
    }
    if method[1] == 2 {
        let credentials = credentials.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SOCKS5 proxy requires credentials",
            )
        })?;
        let username = credentials.username.as_bytes();
        let password = credentials.password.as_bytes();
        let mut request = Zeroizing::new(Vec::with_capacity(3 + username.len() + password.len()));
        request.extend_from_slice(&[1, username.len() as u8]);
        request.extend_from_slice(username);
        request.push(password.len() as u8);
        request.extend_from_slice(password);
        stream.write_all(&request).await?;
        let mut response = [0_u8; 2];
        stream.read_exact(&mut response).await?;
        if response != [1, 0] {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SOCKS5 proxy authentication failed",
            ));
        }
    } else if method[1] != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SOCKS5 proxy selected an unsupported authentication method",
        ));
    }

    let mut request = vec![5, 1, 0];
    append_socks_target(&mut request, target_host, target_port, remote_dns).await?;
    stream.write_all(&request).await?;
    let mut response = [0_u8; 4];
    stream.read_exact(&mut response).await?;
    if response[0] != 5 || response[1] != 0 {
        return Err(io::Error::other(format!(
            "SOCKS5 proxy CONNECT failed with code {}",
            response[1]
        )));
    }
    consume_socks_address(&mut stream, response[3]).await?;
    Ok(Box::new(stream))
}

async fn append_socks_target(
    request: &mut Vec<u8>,
    host: &str,
    port: u16,
    remote_dns: bool,
) -> io::Result<()> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        append_ip(request, ip);
    } else if remote_dns {
        if host.len() > u8::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SOCKS5 target hostname is too long",
            ));
        }
        request.push(3);
        request.push(host.len() as u8);
        request.extend_from_slice(host.as_bytes());
    } else {
        let ip = lookup_host((host, port))
            .await?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "target DNS lookup failed"))?
            .ip();
        append_ip(request, ip);
    }
    request.extend_from_slice(&port.to_be_bytes());
    Ok(())
}

fn append_ip(request: &mut Vec<u8>, ip: IpAddr) {
    match ip {
        IpAddr::V4(ip) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            request.push(4);
            request.extend_from_slice(&ip.octets());
        }
    }
}

async fn consume_socks_address(stream: &mut TcpStream, address_type: u8) -> io::Result<()> {
    let length = match address_type {
        1 => 4,
        3 => {
            let mut length = [0_u8; 1];
            stream.read_exact(&mut length).await?;
            length[0] as usize
        }
        4 => 16,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SOCKS5 proxy returned an invalid address type",
            ));
        }
    };
    let mut address_and_port = vec![0_u8; length + 2];
    stream.read_exact(&mut address_and_port).await?;
    Ok(())
}

async fn read_http_head<R: AsyncRead + Unpin>(stream: &mut R) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let mut data = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before HTTP headers completed",
            ));
        }
        data.extend_from_slice(&buffer[..read]);
        if let Some(end) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            if end > MAX_HEADER_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP proxy headers are too large",
                ));
            }
            let trailing = data.split_off(end);
            return Ok((data, trailing));
        }
        if data.len() > MAX_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP proxy headers are too large",
            ));
        }
    }
}

fn http_response_is_success(response: &[u8]) -> bool {
    std::str::from_utf8(response)
        .ok()
        .and_then(|response| response.lines().next())
        .and_then(|line| line.split_whitespace().nth(1))
        .is_some_and(|status| status == "200")
}

fn parse_authority(authority: &str, default_port: u16) -> io::Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest.split_once("]:").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid IPv6 CONNECT authority",
            )
        })?;
        let port = port
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid CONNECT port"))?;
        return Ok((host.to_string(), port));
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid CONNECT port"))?;
        if host.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CONNECT host is empty",
            ));
        }
        return Ok((host.to_string(), port));
    }
    if authority.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CONNECT host is empty",
        ));
    }
    Ok((authority.to_string(), default_port))
}

fn format_authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_proxy_urls_and_decodes_credentials() {
        let UpstreamProxy::Socks5 {
            remote_dns,
            credentials,
            ..
        } = UpstreamProxy::parse("socks5h://user%40mail:p%40ss@proxy.example:1080").unwrap()
        else {
            panic!("expected SOCKS5 proxy");
        };
        assert!(remote_dns);
        let credentials = credentials.unwrap();
        assert_eq!(credentials.username.as_str(), "user@mail");
        assert_eq!(credentials.password.as_str(), "p@ss");
        assert!(matches!(
            UpstreamProxy::parse("https://proxy.example:8443").unwrap(),
            UpstreamProxy::Http { tls: true, .. }
        ));
    }

    #[tokio::test]
    async fn relays_http_connect_with_basic_proxy_authentication() {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let (head_tx, head_rx) = tokio::sync::oneshot::channel();
        let upstream_task = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let (head, trailing) = read_http_head(&mut socket).await.unwrap();
            assert!(trailing.is_empty());
            head_tx.send(String::from_utf8(head).unwrap()).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let mut payload = [0_u8; 4];
            socket.read_exact(&mut payload).await.unwrap();
            socket.write_all(&payload).await.unwrap();
        });
        let bridge = ProxyBridge::start(&format!(
            "http://user:pass@127.0.0.1:{}",
            upstream_addr.port()
        ))
        .await
        .unwrap();

        timeout(Duration::from_secs(3), async {
            let mut client = TcpStream::connect(bridge.addr()).await.unwrap();
            client
                .write_all(b"CONNECT opencode.ai:443 HTTP/1.1\r\nHost: opencode.ai:443\r\n\r\n")
                .await
                .unwrap();
            let (response, trailing) = read_http_head(&mut client).await.unwrap();
            assert!(http_response_is_success(&response));
            assert!(trailing.is_empty());
            client.write_all(b"ping").await.unwrap();
            let mut echoed = [0_u8; 4];
            client.read_exact(&mut echoed).await.unwrap();
            assert_eq!(&echoed, b"ping");
        })
        .await
        .unwrap();
        let head = head_rx.await.unwrap();
        assert!(head.contains("CONNECT opencode.ai:443 HTTP/1.1"));
        assert!(head.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
        upstream_task.await.unwrap();
    }

    #[tokio::test]
    async fn relays_authenticated_socks5h_connect() {
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let upstream_task = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut greeting = [0_u8; 4];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 2, 0, 2]);
            socket.write_all(&[5, 2]).await.unwrap();
            let mut auth_head = [0_u8; 2];
            socket.read_exact(&mut auth_head).await.unwrap();
            assert_eq!(auth_head, [1, 4]);
            let mut username = [0_u8; 4];
            socket.read_exact(&mut username).await.unwrap();
            let mut password_length = [0_u8; 1];
            socket.read_exact(&mut password_length).await.unwrap();
            let mut password = vec![0_u8; password_length[0] as usize];
            socket.read_exact(&mut password).await.unwrap();
            assert_eq!(&username, b"user");
            assert_eq!(&password, b"pass");
            socket.write_all(&[1, 0]).await.unwrap();

            let mut connect = [0_u8; 5];
            socket.read_exact(&mut connect).await.unwrap();
            assert_eq!(&connect[..4], &[5, 1, 0, 3]);
            let mut hostname = vec![0_u8; connect[4] as usize];
            socket.read_exact(&mut hostname).await.unwrap();
            let mut port = [0_u8; 2];
            socket.read_exact(&mut port).await.unwrap();
            assert_eq!(&hostname, b"opencode.ai");
            assert_eq!(u16::from_be_bytes(port), 443);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            let mut payload = [0_u8; 4];
            socket.read_exact(&mut payload).await.unwrap();
            socket.write_all(&payload).await.unwrap();
        });
        let bridge = ProxyBridge::start(&format!(
            "socks5h://user:pass@127.0.0.1:{}",
            upstream_addr.port()
        ))
        .await
        .unwrap();

        timeout(Duration::from_secs(3), async {
            let mut client = TcpStream::connect(bridge.addr()).await.unwrap();
            client
                .write_all(b"CONNECT opencode.ai:443 HTTP/1.1\r\nHost: opencode.ai:443\r\n\r\n")
                .await
                .unwrap();
            let (response, trailing) = read_http_head(&mut client).await.unwrap();
            assert!(http_response_is_success(&response));
            assert!(trailing.is_empty());
            client.write_all(b"ping").await.unwrap();
            let mut echoed = [0_u8; 4];
            client.read_exact(&mut echoed).await.unwrap();
            assert_eq!(&echoed, b"ping");
        })
        .await
        .unwrap();
        upstream_task.await.unwrap();
    }
}
