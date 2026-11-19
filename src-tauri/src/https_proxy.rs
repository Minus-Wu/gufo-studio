use rcgen::{CertificateParams, DnType, KeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::BTreeSet;
use std::fs;
use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;
use x509_parser::extensions::GeneralName;

pub const DEFAULT_CERT_FILENAME: &str = "gufo-studio.cert.pem";
pub const DEFAULT_KEY_FILENAME: &str = "gufo-studio.key.pem";

/// 去掉 IPv6 字面值外圍的中括號 (供 `TcpListener::bind` / `TcpStream::connect` 使用)。
pub fn strip_ipv6_brackets(host: &str) -> &str {
    let trimmed = host.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed)
}

/// 取得實際傳給 `gufo --host` 與本機監聽器使用的綁定位址；空字串預設回退為 `0.0.0.0`。
pub fn effective_bind_host(host: &str) -> String {
    let stripped = strip_ipv6_brackets(host);
    if stripped.is_empty() {
        "0.0.0.0".to_string()
    } else {
        stripped.to_string()
    }
}

/// 判斷是否為萬用綁定位址 (`0.0.0.0`、`::` 或空字串)。
pub fn is_wildcard_host(host: &str) -> bool {
    matches!(strip_ipv6_brackets(host), "" | "0.0.0.0" | "::")
}

/// 將綁定位址轉換為本機客戶端 (健康檢查、Chat UI、代理上游) 可連線的位址。
/// 在 Windows 上對 `0.0.0.0` 或 `::` 主動連線會失敗，必須轉成 `127.0.0.1` 或 `::1`。
pub fn connect_host(bind_host: &str) -> String {
    match strip_ipv6_brackets(bind_host) {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" => "::1".to_string(),
        other => other.to_string(),
    }
}

/// 格式化為可放入 URL 的主機字串 (IPv6 位址會加上 `[...]`)。
pub fn format_url_host(host: &str) -> String {
    let clean = strip_ipv6_brackets(host);
    if clean.contains(':') {
        format!("[{}]", clean)
    } else {
        clean.to_string()
    }
}

/// 偵測本機對外預設路由介面的區域網路 IPv4 位址 (不實際送出任何封包)。
pub fn detect_local_ipv4() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip.to_string()),
        _ => None,
    }
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// 偵測本機所有非 loopback、非 link-local 的網路介面 IP 位址。
pub fn detect_all_local_ips() -> Vec<String> {
    let mut ips = BTreeSet::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            let ip = iface.ip();
            if !ip.is_loopback() && !ip.is_unspecified() && !is_link_local(ip) {
                ips.insert(ip.to_string());
            }
        }
    }
    if let Some(primary) = detect_local_ipv4() {
        ips.insert(primary);
    }
    ips.into_iter().collect()
}

/// 解析以逗號、分號或空白分隔的自訂 SAN 清單。
pub fn parse_san_list(raw: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for part in raw.split([',', ';', ' ', '\t', '\r', '\n']) {
        let clean = strip_ipv6_brackets(part);
        if !clean.is_empty() && !is_wildcard_host(clean) {
            out.insert(clean.to_string());
        }
    }
    out.into_iter().collect()
}

pub fn default_cert_paths(base_dir: &Path) -> (PathBuf, PathBuf) {
    (
        base_dir.join(DEFAULT_CERT_FILENAME),
        base_dir.join(DEFAULT_KEY_FILENAME),
    )
}

fn collect_san_entries(extra_sans: &[String]) -> Vec<String> {
    let mut sans = BTreeSet::new();
    sans.insert("localhost".to_string());
    sans.insert("127.0.0.1".to_string());
    sans.insert("::1".to_string());

    for ip in detect_all_local_ips() {
        sans.insert(ip);
    }

    for env_key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(name) = std::env::var(env_key) {
            let trimmed = name.trim();
            if !trimmed.is_empty() && !trimmed.contains(char::is_whitespace) {
                sans.insert(trimmed.to_string());
            }
        }
    }

    for entry in extra_sans {
        let clean = strip_ipv6_brackets(entry);
        if !clean.is_empty() && !is_wildcard_host(clean) {
            sans.insert(clean.to_string());
        }
    }

    sans.into_iter().collect()
}

/// 檢查憑證檔是否已包含所有指定的 SAN (IP 與 DNS 名稱)。
pub fn cert_contains_all_sans(cert_path: &Path, required_sans: &[String]) -> bool {
    let required: Vec<String> = required_sans
        .iter()
        .map(|s| strip_ipv6_brackets(s))
        .filter(|s| !s.is_empty() && !is_wildcard_host(s))
        .map(|s| match s.parse::<IpAddr>() {
            Ok(ip) => ip.to_string(),
            Err(_) => s.to_ascii_lowercase(),
        })
        .collect();
    if required.is_empty() {
        return true;
    }

    let Ok(cert_bytes) = fs::read(cert_path) else {
        return false;
    };
    let mut reader = BufReader::new(cert_bytes.as_slice());
    let Some(Ok(first_cert)) = rustls_pemfile::certs(&mut reader).next() else {
        return false;
    };
    let Ok((_, parsed)) = x509_parser::parse_x509_certificate(first_cert.as_ref()) else {
        return false;
    };
    let Ok(Some(san_ext)) = parsed.subject_alternative_name() else {
        return false;
    };

    let mut present = BTreeSet::new();
    for name in &san_ext.value.general_names {
        match name {
            GeneralName::DNSName(dns) => {
                present.insert(dns.to_ascii_lowercase());
            }
            GeneralName::IPAddress(bytes) => {
                if let Ok(octets) = <[u8; 4]>::try_from(*bytes) {
                    present.insert(IpAddr::V4(Ipv4Addr::from(octets)).to_string());
                } else if let Ok(octets) = <[u8; 16]>::try_from(*bytes) {
                    present.insert(IpAddr::V6(Ipv6Addr::from(octets)).to_string());
                }
            }
            _ => {}
        }
    }

    required.iter().all(|req| present.contains(req))
}

/// 產生自簽 TLS 憑證與私鑰並寫入指定路徑。
pub fn generate_self_signed_cert_files(
    cert_path: &Path,
    key_path: &Path,
    extra_sans: &[String],
) -> Result<(), String> {
    let sans = collect_san_entries(extra_sans);
    let mut params = CertificateParams::new(sans)
        .map_err(|e| format!("建立自簽憑證參數失敗: {}", e))?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Gufo Studio Local HTTPS");

    let signing_key = KeyPair::generate().map_err(|e| format!("產生 TLS 金鑰失敗: {}", e))?;
    let cert = params
        .self_signed(&signing_key)
        .map_err(|e| format!("簽署自簽憑證失敗: {}", e))?;

    if let Some(parent) = cert_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("建立憑證目錄 {} 失敗: {}", parent.display(), e))?;
    }
    if let Some(parent) = key_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| format!("建立私鑰目錄 {} 失敗: {}", parent.display(), e))?;
    }

    fs::write(cert_path, cert.pem())
        .map_err(|e| format!("寫入憑證檔 {} 失敗: {}", cert_path.display(), e))?;
    fs::write(key_path, signing_key.serialize_pem())
        .map_err(|e| format!("寫入私鑰檔 {} 失敗: {}", key_path.display(), e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(key_path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

pub fn load_rustls_server_config(
    cert_path: &Path,
    key_path: &Path,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let cert_bytes = fs::read(cert_path)
        .map_err(|e| format!("讀取 HTTPS 憑證檔 {} 失敗: {}", cert_path.display(), e))?;
    let key_bytes = fs::read(key_path)
        .map_err(|e| format!("讀取 HTTPS 私鑰檔 {} 失敗: {}", key_path.display(), e))?;

    let mut cert_reader = BufReader::new(cert_bytes.as_slice());
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析 HTTPS 憑證 PEM {} 失敗: {}", cert_path.display(), e))?;
    if certs.is_empty() {
        return Err(format!("HTTPS 憑證檔 {} 未包含有效的憑證內容", cert_path.display()));
    }

    let mut key_reader = BufReader::new(key_bytes.as_slice());
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| format!("解析 HTTPS 私鑰 PEM {} 失敗: {}", key_path.display(), e))?
        .ok_or_else(|| format!("HTTPS 私鑰檔 {} 未包含有效的私鑰內容", key_path.display()))?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("初始化 TLS 協定版本失敗: {}", e))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("載入 TLS 憑證或私鑰失敗: {}", e))?;

    // 底層 Gufo 伺服器為 HTTP/1.1，固定協商 http/1.1 避免客戶端嘗試 HTTP/2 造成不相容
    config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(Arc::new(config))
}

pub struct PreparedTls {
    pub server_config: Arc<rustls::ServerConfig>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub auto_generated: bool,
    pub newly_created: bool,
}

/// 解析自訂 TLS 憑證，或在未指定時載入/自動建立本機自簽憑證。
pub fn prepare_tls_server_config<F>(
    cert_path_cfg: &str,
    key_path_cfg: &str,
    default_dir: &Path,
    extra_sans: &[String],
    resolve_path: F,
) -> Result<PreparedTls, String>
where
    F: Fn(&str) -> PathBuf,
{
    let cert_trimmed = cert_path_cfg.trim();
    let key_trimmed = key_path_cfg.trim();

    if cert_trimmed.is_empty() != key_trimmed.is_empty() {
        return Err(
            "請同時指定 HTTPS 憑證檔 (Cert) 與私鑰檔 (Key)，或兩者皆留空以自動產生自簽憑證。"
                .to_string(),
        );
    }

    if !cert_trimmed.is_empty() {
        let cert_path = resolve_path(cert_trimmed);
        let key_path = resolve_path(key_trimmed);
        if !cert_path.exists() {
            return Err(format!("找不到 HTTPS 憑證檔案：{}", cert_path.display()));
        }
        if !key_path.exists() {
            return Err(format!("找不到 HTTPS 私鑰檔案：{}", key_path.display()));
        }
        let server_config = load_rustls_server_config(&cert_path, &key_path)?;
        return Ok(PreparedTls {
            server_config,
            cert_path,
            key_path,
            auto_generated: false,
            newly_created: false,
        });
    }

    let (cert_path, key_path) = default_cert_paths(default_dir);
    let mut newly_created = false;

    let existing = if cert_path.exists()
        && key_path.exists()
        && cert_contains_all_sans(&cert_path, extra_sans)
    {
        load_rustls_server_config(&cert_path, &key_path).ok()
    } else {
        None
    };

    let server_config = match existing {
        Some(cfg) => cfg,
        None => {
            generate_self_signed_cert_files(&cert_path, &key_path, extra_sans)?;
            newly_created = true;
            load_rustls_server_config(&cert_path, &key_path)?
        }
    };

    Ok(PreparedTls {
        server_config,
        cert_path,
        key_path,
        auto_generated: true,
        newly_created,
    })
}

pub struct HttpsProxyHandle {
    local_addr: SocketAddr,
    shutdown_tx: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl HttpsProxyHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn shutdown(&mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for HttpsProxyHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const MAX_AUTH_HEADER_BYTES: usize = 64 * 1024;
const AUTH_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn is_options_request(header_bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(header_bytes);
    text.lines()
        .next()
        .map(|line| line.starts_with("OPTIONS "))
        .unwrap_or(false)
}

fn is_authorized_bearer(header_bytes: &[u8], expected_key: &str) -> bool {
    let text = String::from_utf8_lossy(header_bytes);
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("authorization") {
            continue;
        }
        let trimmed = value.trim();
        let Some((scheme, token)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        if scheme.eq_ignore_ascii_case("bearer") && token.trim() == expected_key {
            return true;
        }
    }
    false
}

enum HeadReadResult {
    Complete { buf: Vec<u8>, header_end: usize },
    TooLarge,
    ClosedOrInvalid,
}

async fn read_http_head<S>(stream: &mut S) -> HeadReadResult
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + AUTH_HEADER_READ_TIMEOUT;

    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return HeadReadResult::ClosedOrInvalid;
        }
        let read_res = tokio::time::timeout(deadline - now, stream.read(&mut chunk)).await;
        match read_res {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return HeadReadResult::ClosedOrInvalid,
            Ok(Ok(n)) => {
                if buf.len() + n > MAX_AUTH_HEADER_BYTES {
                    return HeadReadResult::TooLarge;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = find_header_end(&buf) {
                    return HeadReadResult::Complete { buf, header_end };
                }
            }
        }
    }
}

async fn send_http_response<S>(stream: &mut S, raw_response: &[u8])
where
    S: AsyncWriteExt + Unpin,
{
    let _ = stream.write_all(raw_response).await;
    let _ = stream.shutdown().await;
}

async fn send_upstream_unavailable<S>(stream: &mut S)
where
    S: AsyncWriteExt + Unpin,
{
    let body = r#"{"error":{"message":"Gufo inference server is loading or unavailable"}}"#;
    let resp = format!(
        "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    send_http_response(stream, resp.as_bytes()).await;
}

/// 啟動內嵌 HTTPS (TLS) 反向代理，將入站 TLS 連線轉發至本機 Gufo HTTP 伺服器。
/// 若提供非空的 `api_key`，則於轉發前驗證 `Authorization: Bearer <KEY>` 標頭。
pub async fn start_https_proxy(
    bind_host: &str,
    https_port: u16,
    upstream_host: &str,
    upstream_port: u16,
    server_config: Arc<rustls::ServerConfig>,
    api_key: Option<String>,
) -> Result<HttpsProxyHandle, String> {
    let bind_ip = effective_bind_host(bind_host);
    let listener = TcpListener::bind((bind_ip.as_str(), https_port))
        .await
        .map_err(|e| format!("無法在 {}:{} 啟動 HTTPS 代理監聽: {}", bind_ip, https_port, e))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("無法取得 HTTPS 監聽位址: {}", e))?;

    let upstream_target = (strip_ipv6_brackets(upstream_host).to_string(), upstream_port);
    let expected_api_key = api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .map(Arc::new);
    let acceptor = TlsAcceptor::from(server_config);
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((tcp_stream, _peer)) => {
                            let acceptor = acceptor.clone();
                            let upstream_target = upstream_target.clone();
                            let expected_api_key = expected_api_key.clone();
                            let mut conn_shutdown = shutdown_rx.clone();
                            tokio::spawn(async move {
                                let _ = tcp_stream.set_nodelay(true);
                                let mut tls_stream = match acceptor.accept(tcp_stream).await {
                                    Ok(s) => s,
                                    Err(_) => return,
                                };

                                let prefetched = if let Some(ref key) = expected_api_key {
                                    match read_http_head(&mut tls_stream).await {
                                        HeadReadResult::Complete { buf, header_end } => {
                                            let header_slice = &buf[..header_end];
                                            if !is_authorized_bearer(header_slice, key) {
                                                if is_options_request(header_slice) {
                                                    let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS, DELETE\r\nAccess-Control-Allow-Headers: Authorization, Content-Type\r\nConnection: close\r\n\r\n";
                                                    send_http_response(&mut tls_stream, resp.as_bytes()).await;
                                                } else {
                                                    let body = r#"{"error":{"message":"Invalid or missing Bearer API key","type":"invalid_request_error","code":"invalid_api_key"}}"#;
                                                    let resp = format!(
                                                        "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nWWW-Authenticate: Bearer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                                        body.len(),
                                                        body
                                                    );
                                                    send_http_response(&mut tls_stream, resp.as_bytes()).await;
                                                }
                                                return;
                                            }
                                            Some(buf)
                                        }
                                        HeadReadResult::TooLarge => {
                                            let body = r#"{"error":{"message":"Request header too large","type":"invalid_request_error","code":"header_too_large"}}"#;
                                            let resp = format!(
                                                "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                                body.len(),
                                                body
                                            );
                                            send_http_response(&mut tls_stream, resp.as_bytes()).await;
                                            return;
                                        }
                                        HeadReadResult::ClosedOrInvalid => return,
                                    }
                                } else {
                                    None
                                };

                                match TcpStream::connect((upstream_target.0.as_str(), upstream_target.1)).await {
                                    Ok(mut upstream) => {
                                        let _ = upstream.set_nodelay(true);
                                        if let Some(initial_bytes) = prefetched {
                                            if upstream.write_all(&initial_bytes).await.is_err() {
                                                return;
                                            }
                                        }
                                        tokio::select! {
                                            _ = tokio::io::copy_bidirectional(&mut tls_stream, &mut upstream) => {}
                                            _ = conn_shutdown.changed() => {}
                                        }
                                    }
                                    Err(_) => {
                                        send_upstream_unavailable(&mut tls_stream).await;
                                    }
                                }
                            });
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        }
    });

    Ok(HttpsProxyHandle {
        local_addr,
        shutdown_tx,
        task: Some(task),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_helpers() {
        assert_eq!(effective_bind_host(""), "0.0.0.0");
        assert_eq!(effective_bind_host("  0.0.0.0 "), "0.0.0.0");
        assert_eq!(effective_bind_host("[::]"), "::");
        assert!(is_wildcard_host(""));
        assert!(is_wildcard_host("0.0.0.0"));
        assert!(is_wildcard_host("::"));
        assert!(is_wildcard_host("[::]"));
        assert!(!is_wildcard_host("127.0.0.1"));

        assert_eq!(connect_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(connect_host(""), "127.0.0.1");
        assert_eq!(connect_host("::"), "::1");
        assert_eq!(connect_host("[::]"), "::1");
        assert_eq!(connect_host("192.168.1.10"), "192.168.1.10");

        assert_eq!(format_url_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(format_url_host("::1"), "[::1]");
        assert_eq!(format_url_host("[::1]"), "[::1]");
    }

    #[test]
    fn test_prepare_tls_rejects_partial_custom_paths() {
        let dir = std::env::temp_dir().join("gufo_test_partial_tls");
        let err = prepare_tls_server_config("cert.pem", "", &dir, &[], |s| PathBuf::from(s))
            .err()
            .expect("should reject cert without key");
        assert!(err.contains("同時指定"));
    }

    #[test]
    fn test_auto_generate_and_reuse_self_signed_cert() {
        let dir = std::env::temp_dir().join(format!(
            "gufo_tls_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let first = prepare_tls_server_config("", "", &dir, &["192.168.10.20".to_string()], |s| PathBuf::from(s))
            .expect("should auto-generate cert");
        assert!(first.auto_generated);
        assert!(first.newly_created);
        assert!(first.cert_path.exists());
        assert!(first.key_path.exists());

        let second = prepare_tls_server_config("", "", &dir, &[], |s| PathBuf::from(s))
            .expect("should reuse existing cert");
        assert!(second.auto_generated);
        assert!(!second.newly_created);
        assert!(cert_contains_all_sans(&second.cert_path, &["192.168.10.20".to_string()]));

        // 若之後新增明確要求的 SAN (如更換服務 IP)，應自動重新簽發包含新 SAN 的憑證
        let third = prepare_tls_server_config(
            "",
            "",
            &dir,
            &["203.0.113.42".to_string()],
            |s| PathBuf::from(s),
        )
        .expect("should regenerate cert when new required SAN is added");
        assert!(third.auto_generated);
        assert!(third.newly_created);
        assert!(cert_contains_all_sans(&third.cert_path, &["203.0.113.42".to_string()]));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_san_list() {
        assert_eq!(
            parse_san_list("192.168.122.122, gufo.local; [::1] 0.0.0.0"),
            vec![
                "192.168.122.122".to_string(),
                "::1".to_string(),
                "gufo.local".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn test_https_proxy_forwards_to_http_upstream() {
        let dir = std::env::temp_dir().join(format!(
            "gufo_proxy_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let prepared = prepare_tls_server_config("", "", &dir, &[], |s| PathBuf::from(s))
            .expect("generate cert");

        // 啟動模擬的上游 Gufo HTTP 伺服器
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = upstream.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"status":"ok"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });

        let mut handle = start_https_proxy(
            "127.0.0.1",
            0,
            "127.0.0.1",
            upstream_port,
            prepared.server_config,
            None,
        )
        .await
        .expect("start https proxy");
        let https_port = handle.local_addr().port();

        let cert_pem = fs::read(&prepared.cert_path).unwrap();
        let root_cert = reqwest::Certificate::from_pem(&cert_pem).unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(root_cert)
            .build()
            .unwrap();

        let resp = client
            .get(format!("https://127.0.0.1:{}/ready", https_port))
            .send()
            .await
            .expect("https request should succeed");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.text().await.unwrap(), r#"{"status":"ok"}"#);

        handle.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_https_proxy_enforces_api_key_when_configured() {
        let dir = std::env::temp_dir().join(format!(
            "gufo_proxy_auth_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let prepared = prepare_tls_server_config("", "", &dir, &[], |s| PathBuf::from(s))
            .expect("generate cert");

        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = upstream.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let body = r#"{"status":"authorized"}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });

        let mut handle = start_https_proxy(
            "127.0.0.1",
            0,
            "127.0.0.1",
            upstream_port,
            prepared.server_config,
            Some("sk-test-123".to_string()),
        )
        .await
        .expect("start https proxy with auth");
        let https_port = handle.local_addr().port();

        let cert_pem = fs::read(&prepared.cert_path).unwrap();
        let root_cert = reqwest::Certificate::from_pem(&cert_pem).unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(root_cert)
            .build()
            .unwrap();

        let url = format!("https://127.0.0.1:{}/v1/models", https_port);

        // 未帶金鑰 -> 401
        let unauth_resp = client.get(&url).send().await.expect("unauth request");
        assert_eq!(unauth_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        // 帶錯誤金鑰 -> 401
        let wrong_resp = client
            .get(&url)
            .bearer_auth("sk-wrong")
            .send()
            .await
            .expect("wrong token request");
        assert_eq!(wrong_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

        // 帶正確金鑰 -> 200
        let ok_resp = client
            .get(&url)
            .bearer_auth("sk-test-123")
            .send()
            .await
            .expect("authorized request");
        assert_eq!(ok_resp.status(), reqwest::StatusCode::OK);
        assert_eq!(ok_resp.text().await.unwrap(), r#"{"status":"authorized"}"#);

        handle.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }
}
