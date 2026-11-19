use rcgen::{CertificateParams, DnType, KeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::BTreeSet;
use std::fs;
use std::io::BufReader;
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

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
        std::net::IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip.to_string()),
        _ => None,
    }
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

    if let Some(lan_ip) = detect_local_ipv4() {
        sans.insert(lan_ip);
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

    let existing = if cert_path.exists() && key_path.exists() {
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

/// 啟動內嵌 HTTPS (TLS) 反向代理，將入站 TLS 連線轉發至本機 Gufo HTTP 伺服器。
pub async fn start_https_proxy(
    bind_host: &str,
    https_port: u16,
    upstream_host: &str,
    upstream_port: u16,
    server_config: Arc<rustls::ServerConfig>,
) -> Result<HttpsProxyHandle, String> {
    let bind_ip = effective_bind_host(bind_host);
    let listener = TcpListener::bind((bind_ip.as_str(), https_port))
        .await
        .map_err(|e| format!("無法在 {}:{} 啟動 HTTPS 代理監聽: {}", bind_ip, https_port, e))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("無法取得 HTTPS 監聽位址: {}", e))?;

    let upstream_target = (strip_ipv6_brackets(upstream_host).to_string(), upstream_port);
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
                            let mut conn_shutdown = shutdown_rx.clone();
                            tokio::spawn(async move {
                                let _ = tcp_stream.set_nodelay(true);
                                let mut tls_stream = match acceptor.accept(tcp_stream).await {
                                    Ok(s) => s,
                                    Err(_) => return,
                                };

                                match TcpStream::connect((upstream_target.0.as_str(), upstream_target.1)).await {
                                    Ok(mut upstream) => {
                                        let _ = upstream.set_nodelay(true);
                                        tokio::select! {
                                            _ = tokio::io::copy_bidirectional(&mut tls_stream, &mut upstream) => {}
                                            _ = conn_shutdown.changed() => {}
                                        }
                                    }
                                    Err(_) => {
                                        let body = r#"{"error":{"message":"Gufo inference server is loading or unavailable"}}"#;
                                        let resp = format!(
                                            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                            body.len(),
                                            body
                                        );
                                        let _ = tls_stream.write_all(resp.as_bytes()).await;
                                        let _ = tls_stream.shutdown().await;
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
    use tokio::io::AsyncReadExt;

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

        let _ = fs::remove_dir_all(&dir);
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
}
