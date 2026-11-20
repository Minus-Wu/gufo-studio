use rcgen::{CertificateParams, DnType, KeyPair};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::BTreeSet;
use std::fs;
use std::io::BufReader;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
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
const MAX_CHAT_REQUEST_REWRITE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CHAT_RESPONSE_REWRITE_BYTES: usize = 32 * 1024 * 1024;

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn parse_request_line(header_bytes: &[u8]) -> Option<(String, String)> {
    let text = String::from_utf8_lossy(header_bytes);
    let first = text.split("\r\n").next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let path = target.split('?').next().unwrap_or(target).to_string();
    Some((method, path))
}

fn is_options_request(header_bytes: &[u8]) -> bool {
    parse_request_line(header_bytes)
        .map(|(method, _)| method.eq_ignore_ascii_case("OPTIONS"))
        .unwrap_or(false)
}

fn is_chat_completions_post(header_bytes: &[u8]) -> bool {
    parse_request_line(header_bytes)
        .map(|(method, path)| method.eq_ignore_ascii_case("POST") && path == "/v1/chat/completions")
        .unwrap_or(false)
}

fn find_header_value(header_bytes: &[u8], target_name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(header_bytes);
    for line in text.split("\r\n").skip(1) {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(target_name) {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn is_chunked_encoding(header_bytes: &[u8]) -> bool {
    find_header_value(header_bytes, "transfer-encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
}

fn has_expect_100_continue(header_bytes: &[u8]) -> bool {
    find_header_value(header_bytes, "expect")
        .map(|v| v.to_ascii_lowercase().contains("100-continue"))
        .unwrap_or(false)
}

fn response_status_is_200(header_bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(header_bytes);
    let Some(first) = text.split("\r\n").next() else {
        return false;
    };
    first
        .split_whitespace()
        .nth(1)
        .map(|code| code == "200")
        .unwrap_or(false)
}

fn rewrite_http_headers(
    header_bytes: &[u8],
    new_content_length: Option<usize>,
    strip_chunked: bool,
    strip_expect: bool,
) -> Vec<u8> {
    let text = String::from_utf8_lossy(header_bytes);
    let mut lines = text.split("\r\n");
    let mut out = String::with_capacity(header_bytes.len() + 32);
    if let Some(first) = lines.next() {
        out.push_str(first);
        out.push_str("\r\n");
    }
    let mut wrote_content_length = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, _)) = line.split_once(':') {
            let trimmed = name.trim();
            if strip_chunked && trimmed.eq_ignore_ascii_case("transfer-encoding") {
                continue;
            }
            if strip_expect && trimmed.eq_ignore_ascii_case("expect") {
                continue;
            }
            if new_content_length.is_some() && trimmed.eq_ignore_ascii_case("content-length") {
                if !wrote_content_length {
                    out.push_str(&format!("Content-Length: {}\r\n", new_content_length.unwrap()));
                    wrote_content_length = true;
                }
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    if let Some(len) = new_content_length {
        if !wrote_content_length {
            out.push_str(&format!("Content-Length: {}\r\n", len));
        }
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// 針對 `POST /v1/chat/completions` 的串流請求，若未指定 `stream_options.include_usage`，
/// 自動補上 `{"include_usage": true}`，確保底層 Gufo 必回傳含 `usage` 與 `prompt_tokens_details` 的終端 chunk。
pub fn maybe_inject_include_usage(body: &[u8]) -> Option<Vec<u8>> {
    let mut val: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = val.as_object_mut()?;
    if obj.get("stream").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }

    match obj.get_mut("stream_options") {
        Some(serde_json::Value::Object(opts)) => {
            if matches!(opts.get("include_usage"), Some(serde_json::Value::Bool(_))) {
                return None;
            }
            opts.insert("include_usage".to_string(), serde_json::Value::Bool(true));
        }
        None | Some(serde_json::Value::Null) => {
            let mut opts = serde_json::Map::new();
            opts.insert("include_usage".to_string(), serde_json::Value::Bool(true));
            obj.insert("stream_options".to_string(), serde_json::Value::Object(opts));
        }
        _ => return None,
    }

    serde_json::to_vec(&val).ok()
}

/// 補齊 OpenAI `/v1/chat/completions` 回應中的 `usage` 快取寫入與細節欄位：
/// - `usage.prompt_tokens_details.cache_write_tokens`
/// - `usage.cache_write_tokens` / `usage.cache_creation_input_tokens`
/// 數值來源優先取 `usage.gufo.prefill_tokens`，其次 `timings.prompt_n`，最後以 `prompt_tokens - cached_tokens` 推算。
pub fn enrich_chat_completion_usage(val: &mut serde_json::Value) -> bool {
    let timings_prompt_n = val
        .get("timings")
        .and_then(|t| t.get("prompt_n"))
        .and_then(|v| v.as_u64());

    let Some(usage) = val.get_mut("usage").and_then(|v| v.as_object_mut()) else {
        return false;
    };

    let prompt_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64());
    let cached_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("cached_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    let write_tokens = usage
        .get("gufo")
        .and_then(|g| g.get("prefill_tokens"))
        .and_then(|v| v.as_u64())
        .or(timings_prompt_n)
        .or_else(|| prompt_tokens.map(|p| p.saturating_sub(cached_tokens)));

    let Some(write_tokens) = write_tokens else {
        return false;
    };

    let mut changed = false;

    if !matches!(usage.get("prompt_tokens_details"), Some(serde_json::Value::Object(_))) {
        let mut details = serde_json::Map::new();
        details.insert("cached_tokens".to_string(), serde_json::Value::from(cached_tokens));
        usage.insert("prompt_tokens_details".to_string(), serde_json::Value::Object(details));
        changed = true;
    }

    if let Some(details) = usage
        .get_mut("prompt_tokens_details")
        .and_then(|d| d.as_object_mut())
    {
        if !details.contains_key("cached_tokens") {
            details.insert("cached_tokens".to_string(), serde_json::Value::from(cached_tokens));
            changed = true;
        }
        let need_write = match details.get("cache_write_tokens").and_then(|v| v.as_u64()) {
            None => true,
            Some(0) if write_tokens > 0 => true,
            _ => false,
        };
        if need_write {
            details.insert("cache_write_tokens".to_string(), serde_json::Value::from(write_tokens));
            changed = true;
        }
    }

    for key in ["cache_write_tokens", "cache_creation_input_tokens"] {
        let need_key = match usage.get(key).and_then(|v| v.as_u64()) {
            None => true,
            Some(0) if write_tokens > 0 => true,
            _ => false,
        };
        if need_key {
            usage.insert(key.to_string(), serde_json::Value::from(write_tokens));
            changed = true;
        }
    }

    changed
}

fn enrich_sse_line(line: &[u8]) -> Option<Vec<u8>> {
    if !line.starts_with(b"data:") {
        return None;
    }
    let has_crlf = line.ends_with(b"\r\n");
    let has_lf = line.ends_with(b"\n");
    let stripped = if has_crlf {
        &line[..line.len() - 2]
    } else if has_lf {
        &line[..line.len() - 1]
    } else {
        line
    };
    let payload = std::str::from_utf8(&stripped[5..]).ok()?.trim();
    if payload.is_empty() || payload == "[DONE]" || !payload.contains("\"usage\"") {
        return None;
    }
    let mut val: serde_json::Value = serde_json::from_str(payload).ok()?;
    if !enrich_chat_completion_usage(&mut val) {
        return None;
    }
    let serialized = serde_json::to_string(&val).ok()?;
    let ending = if has_crlf {
        "\r\n"
    } else if has_lf {
        "\n"
    } else {
        ""
    };
    Some(format!("data: {}{}", serialized, ending).into_bytes())
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

async fn read_http_head<S>(stream: &mut S, timeout: Option<Duration>) -> HeadReadResult
where
    S: AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let deadline = timeout.map(|t| tokio::time::Instant::now() + t);

    loop {
        let read_res = if let Some(dl) = deadline {
            let now = tokio::time::Instant::now();
            if now >= dl {
                return HeadReadResult::ClosedOrInvalid;
            }
            match tokio::time::timeout(dl - now, stream.read(&mut chunk)).await {
                Ok(r) => r,
                Err(_) => return HeadReadResult::ClosedOrInvalid,
            }
        } else {
            stream.read(&mut chunk).await
        };

        match read_res {
            Ok(0) | Err(_) => return HeadReadResult::ClosedOrInvalid,
            Ok(n) => {
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

async fn read_crlf_line<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut line = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte).await? {
            0 => {
                return Ok(if line.is_empty() { None } else { Some(line) });
            }
            _ => {
                line.push(byte[0]);
                if line.len() > 8192 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "chunk header line too long",
                    ));
                }
                if line.ends_with(b"\r\n") {
                    line.truncate(line.len() - 2);
                    return Ok(Some(line));
                }
            }
        }
    }
}

async fn read_next_http_chunk<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let Some(size_line) = read_crlf_line(reader).await? else {
        return Ok(None);
    };
    let size_str = String::from_utf8_lossy(&size_line);
    let hex_part = size_str.split(';').next().unwrap_or("").trim();
    let chunk_size = usize::from_str_radix(hex_part, 16)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid chunk size"))?;

    if chunk_size == 0 {
        // 消耗可選的 trailer headers 直到空白行
        loop {
            match read_crlf_line(reader).await? {
                None => break,
                Some(line) if line.is_empty() => break,
                Some(_) => continue,
            }
        }
        return Ok(None);
    }

    let mut data = vec![0u8; chunk_size];
    reader.read_exact(&mut data).await?;
    let mut crlf = [0u8; 2];
    reader.read_exact(&mut crlf).await?;
    Ok(Some(data))
}

async fn read_chunked_body<R>(reader: &mut R, max_bytes: usize) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    while let Some(chunk) = read_next_http_chunk(reader).await? {
        if out.len() + chunk.len() > max_bytes {
            return Ok(None);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Some(out))
}

async fn read_to_end_bounded<R>(reader: &mut R, max_bytes: usize) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(Some(out));
        }
        if out.len() + n > max_bytes {
            return Ok(None);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

async fn write_http_chunk<W>(writer: &mut W, data: &[u8]) -> std::io::Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    if data.is_empty() {
        return Ok(());
    }
    let prefix = format!("{:x}\r\n", data.len());
    writer.write_all(prefix.as_bytes()).await?;
    writer.write_all(data).await?;
    writer.write_all(b"\r\n").await
}

async fn proxy_chat_completions<S>(
    tls_stream: &mut S,
    upstream: &mut TcpStream,
    req_head: Vec<u8>,
    initial_req_body: Vec<u8>,
    conn_shutdown: &mut watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWriteExt + Unpin,
{
    let had_expect = has_expect_100_continue(&req_head);
    if had_expect {
        if tls_stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.is_err() {
            return;
        }
        let _ = tls_stream.flush().await;
    }

    let req_chunked = is_chunked_encoding(&req_head);
    let content_length = find_header_value(&req_head, "content-length")
        .and_then(|v| v.parse::<usize>().ok());

    let mut fallback_raw = false;
    if req_chunked {
        let body_res = {
            let mut reader = std::io::Cursor::new(&initial_req_body).chain(&mut *tls_stream);
            read_chunked_body(&mut reader, MAX_CHAT_REQUEST_REWRITE_BYTES).await
        };
        match body_res {
            Ok(Some(req_body)) => {
                let final_body = maybe_inject_include_usage(&req_body).unwrap_or(req_body);
                let new_head = rewrite_http_headers(&req_head, Some(final_body.len()), true, true);
                if upstream.write_all(&new_head).await.is_err()
                    || upstream.write_all(&final_body).await.is_err()
                {
                    return;
                }
            }
            _ => return,
        }
    } else if let Some(len) = content_length {
        if len <= MAX_CHAT_REQUEST_REWRITE_BYTES {
            let mut req_body = vec![0u8; len];
            let read_ok = {
                let mut reader = std::io::Cursor::new(&initial_req_body).chain(&mut *tls_stream);
                reader.read_exact(&mut req_body).await.is_ok()
            };
            if !read_ok {
                return;
            }
            if let Some(modified) = maybe_inject_include_usage(&req_body) {
                let new_head = rewrite_http_headers(&req_head, Some(modified.len()), false, true);
                if upstream.write_all(&new_head).await.is_err()
                    || upstream.write_all(&modified).await.is_err()
                {
                    return;
                }
            } else {
                let head_to_send = if had_expect {
                    rewrite_http_headers(&req_head, None, false, true)
                } else {
                    req_head
                };
                if upstream.write_all(&head_to_send).await.is_err()
                    || upstream.write_all(&req_body).await.is_err()
                {
                    return;
                }
            }
        } else {
            let head_to_send = if had_expect {
                rewrite_http_headers(&req_head, None, false, true)
            } else {
                req_head
            };
            if upstream.write_all(&head_to_send).await.is_err()
                || upstream.write_all(&initial_req_body).await.is_err()
            {
                return;
            }
            fallback_raw = true;
        }
    } else {
        let head_to_send = if had_expect {
            rewrite_http_headers(&req_head, None, false, true)
        } else {
            req_head
        };
        if upstream.write_all(&head_to_send).await.is_err()
            || upstream.write_all(&initial_req_body).await.is_err()
        {
            return;
        }
    }

    if fallback_raw {
        tokio::select! {
            _ = tokio::io::copy_bidirectional(tls_stream, upstream) => {}
            _ = conn_shutdown.changed() => {}
        }
        return;
    }

    let _ = upstream.flush().await;

    // 讀取上游回應標頭（不設固定逾時，避免超長 Prompt Prefill 階段被誤中斷）
    let resp_head_res = tokio::select! {
        res = read_http_head(upstream, None) => res,
        _ = conn_shutdown.changed() => return,
    };

    let (resp_buf, resp_header_end) = match resp_head_res {
        HeadReadResult::Complete { buf, header_end } => (buf, header_end),
        _ => return,
    };
    let resp_head = resp_buf[..resp_header_end].to_vec();
    let initial_resp_body = resp_buf[resp_header_end..].to_vec();

    let is_200 = response_status_is_200(&resp_head);
    let content_type = find_header_value(&resp_head, "content-type")
        .unwrap_or_default()
        .to_ascii_lowercase();

    if is_200 && content_type.starts_with("text/event-stream") {
        if tls_stream.write_all(&resp_head).await.is_err() {
            return;
        }
        let _ = tls_stream.flush().await;

        let resp_chunked = is_chunked_encoding(&resp_head);
        let mut resp_reader = std::io::Cursor::new(initial_resp_body).chain(&mut *upstream);
        let mut line_buf: Vec<u8> = Vec::with_capacity(4096);
        let mut raw_buf = [0u8; 4096];

        loop {
            let piece_opt = if resp_chunked {
                tokio::select! {
                    r = read_next_http_chunk(&mut resp_reader) => match r {
                        Ok(p) => p,
                        Err(_) => return,
                    },
                    _ = conn_shutdown.changed() => return,
                }
            } else {
                tokio::select! {
                    r = resp_reader.read(&mut raw_buf) => match r {
                        Ok(0) => None,
                        Ok(n) => Some(raw_buf[..n].to_vec()),
                        Err(_) => return,
                    },
                    _ = conn_shutdown.changed() => return,
                }
            };

            let Some(piece) = piece_opt else {
                break;
            };
            line_buf.extend_from_slice(&piece);

            let mut out_batch = Vec::with_capacity(piece.len() + 128);
            while let Some(nl_pos) = line_buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = line_buf.drain(..=nl_pos).collect();
                if let Some(enriched) = enrich_sse_line(&line) {
                    out_batch.extend_from_slice(&enriched);
                } else {
                    out_batch.extend_from_slice(&line);
                }
            }

            if !out_batch.is_empty() {
                let write_res = if resp_chunked {
                    write_http_chunk(tls_stream, &out_batch).await
                } else {
                    tls_stream.write_all(&out_batch).await
                };
                if write_res.is_err() {
                    return;
                }
                let _ = tls_stream.flush().await;
            }
        }

        if !line_buf.is_empty() {
            let final_line = enrich_sse_line(&line_buf).unwrap_or(line_buf);
            let write_res = if resp_chunked {
                write_http_chunk(tls_stream, &final_line).await
            } else {
                tls_stream.write_all(&final_line).await
            };
            if write_res.is_err() {
                return;
            }
        }
        if resp_chunked {
            let _ = tls_stream.write_all(b"0\r\n\r\n").await;
        }
        let _ = tls_stream.flush().await;
        let _ = tls_stream.shutdown().await;
        return;
    }

    if is_200 && content_type.starts_with("application/json") {
        let resp_chunked = is_chunked_encoding(&resp_head);
        let resp_len = find_header_value(&resp_head, "content-length")
            .and_then(|v| v.parse::<usize>().ok());

        if resp_len.map(|l| l <= MAX_CHAT_RESPONSE_REWRITE_BYTES).unwrap_or(true) {
            let body_opt = {
                let mut resp_reader = std::io::Cursor::new(&initial_resp_body).chain(&mut *upstream);
                if resp_chunked {
                    read_chunked_body(&mut resp_reader, MAX_CHAT_RESPONSE_REWRITE_BYTES)
                        .await
                        .ok()
                        .flatten()
                } else if let Some(len) = resp_len {
                    let mut body = vec![0u8; len];
                    if resp_reader.read_exact(&mut body).await.is_ok() {
                        Some(body)
                    } else {
                        None
                    }
                } else {
                    read_to_end_bounded(&mut resp_reader, MAX_CHAT_RESPONSE_REWRITE_BYTES)
                        .await
                        .ok()
                        .flatten()
                }
            };

            if let Some(resp_body) = body_opt {
                let final_body = match serde_json::from_slice::<serde_json::Value>(&resp_body) {
                    Ok(mut val) => {
                        if enrich_chat_completion_usage(&mut val) {
                            serde_json::to_vec(&val).unwrap_or(resp_body)
                        } else {
                            resp_body
                        }
                    }
                    Err(_) => resp_body,
                };
                let new_head = rewrite_http_headers(&resp_head, Some(final_body.len()), true, false);
                let _ = tls_stream.write_all(&new_head).await;
                let _ = tls_stream.write_all(&final_body).await;
                let _ = tls_stream.flush().await;
                let _ = tls_stream.shutdown().await;
            }
            return;
        }
    }

    // 非 200 或其他 Content-Type：直接透傳
    if tls_stream.write_all(&resp_head).await.is_err()
        || tls_stream.write_all(&initial_resp_body).await.is_err()
    {
        return;
    }
    tokio::select! {
        _ = tokio::io::copy_bidirectional(tls_stream, upstream) => {}
        _ = conn_shutdown.changed() => {}
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
/// - 若提供非空的 `api_key`，於轉發前驗證 `Authorization: Bearer <KEY>` 標頭。
/// - 針對 `POST /v1/chat/completions` 自動補齊 `stream_options.include_usage: true`（若串流未帶）
///   並於回應補齊 OpenAI 相容的 `usage.prompt_tokens_details.cache_write_tokens` / `cache_creation_input_tokens`。
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

                                let (buf, header_end) = match read_http_head(
                                    &mut tls_stream,
                                    Some(AUTH_HEADER_READ_TIMEOUT),
                                )
                                .await
                                {
                                    HeadReadResult::Complete { buf, header_end } => (buf, header_end),
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
                                };

                                let header_slice = &buf[..header_end];
                                if let Some(ref key) = expected_api_key {
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
                                }

                                let is_chat_post = is_chat_completions_post(header_slice);

                                match TcpStream::connect((upstream_target.0.as_str(), upstream_target.1)).await {
                                    Ok(mut upstream) => {
                                        let _ = upstream.set_nodelay(true);
                                        if is_chat_post {
                                            let req_head = buf[..header_end].to_vec();
                                            let initial_body = buf[header_end..].to_vec();
                                            proxy_chat_completions(
                                                &mut tls_stream,
                                                &mut upstream,
                                                req_head,
                                                initial_body,
                                                &mut conn_shutdown,
                                            )
                                            .await;
                                        } else {
                                            if upstream.write_all(&buf).await.is_err() {
                                                return;
                                            }
                                            tokio::select! {
                                                _ = tokio::io::copy_bidirectional(&mut tls_stream, &mut upstream) => {}
                                                _ = conn_shutdown.changed() => {}
                                            }
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

    #[test]
    fn test_maybe_inject_include_usage() {
        // stream=true 且未帶 stream_options -> 自動注入
        let req = br#"{"model":"flash-next","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;
        let injected = maybe_inject_include_usage(req).expect("should inject include_usage");
        let parsed: serde_json::Value = serde_json::from_slice(&injected).unwrap();
        assert_eq!(parsed["stream_options"]["include_usage"], serde_json::json!(true));

        // stream=true 且帶空 stream_options -> 補入 include_usage: true
        let req_empty_opts = br#"{"model":"flash-next","stream":true,"stream_options":{}}"#;
        let injected2 = maybe_inject_include_usage(req_empty_opts).expect("should inject into empty object");
        let parsed2: serde_json::Value = serde_json::from_slice(&injected2).unwrap();
        assert_eq!(parsed2["stream_options"]["include_usage"], serde_json::json!(true));

        // 明確指定 false -> 尊重客戶端設定不覆寫
        let req_false = br#"{"model":"flash-next","stream":true,"stream_options":{"include_usage":false}}"#;
        assert!(maybe_inject_include_usage(req_false).is_none());

        // 非串流 -> 不修改
        let req_non_stream = br#"{"model":"flash-next","stream":false}"#;
        assert!(maybe_inject_include_usage(req_non_stream).is_none());
    }

    #[test]
    fn test_enrich_chat_completion_usage() {
        let mut resp = serde_json::json!({
            "id": "chatcmpl-1",
            "usage": {
                "prompt_tokens": 20000,
                "completion_tokens": 120,
                "total_tokens": 20120,
                "prompt_tokens_details": { "cached_tokens": 19200 },
                "completion_tokens_details": { "reasoning_tokens": 15 },
                "cached_tokens": 19200,
                "gufo": { "prefill_tokens": 800, "cache_hit": true }
            },
            "timings": { "prompt_n": 800, "cache_n": 19200 }
        });

        assert!(enrich_chat_completion_usage(&mut resp));
        assert_eq!(resp["usage"]["prompt_tokens_details"]["cached_tokens"], 19200);
        assert_eq!(resp["usage"]["prompt_tokens_details"]["cache_write_tokens"], 800);
        assert_eq!(resp["usage"]["completion_tokens_details"]["reasoning_tokens"], 15);
        assert_eq!(resp["usage"]["cache_write_tokens"], 800);
        assert_eq!(resp["usage"]["cache_creation_input_tokens"], 800);

        // 100% cache hit -> cache_write_tokens 為 0
        let mut hit_resp = serde_json::json!({
            "usage": {
                "prompt_tokens": 5000,
                "completion_tokens": 20,
                "prompt_tokens_details": { "cached_tokens": 5000 },
                "gufo": { "prefill_tokens": 0 }
            }
        });
        assert!(enrich_chat_completion_usage(&mut hit_resp));
        assert_eq!(hit_resp["usage"]["prompt_tokens_details"]["cache_write_tokens"], 0);
        assert_eq!(hit_resp["usage"]["cache_creation_input_tokens"], 0);
    }

    #[tokio::test]
    async fn test_https_proxy_enriches_non_streaming_chat_completions() {
        let dir = std::env::temp_dir().join(format!(
            "gufo_proxy_chat_json_{}_{}",
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
            if let Ok((mut sock, _)) = upstream.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let body = r#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":1000,"completion_tokens":10,"total_tokens":1010,"prompt_tokens_details":{"cached_tokens":200},"completion_tokens_details":{"reasoning_tokens":0},"cached_tokens":200,"gufo":{"prefill_tokens":800}},"timings":{"prompt_n":800,"cache_n":200}}"#;
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
            Some("sk-chat-key".to_string()),
        )
        .await
        .expect("start proxy");
        let https_port = handle.local_addr().port();

        let cert_pem = fs::read(&prepared.cert_path).unwrap();
        let root_cert = reqwest::Certificate::from_pem(&cert_pem).unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(root_cert)
            .build()
            .unwrap();

        let resp = client
            .post(format!("https://127.0.0.1:{}/v1/chat/completions", https_port))
            .bearer_auth("sk-chat-key")
            .json(&serde_json::json!({
                "model": "flash-next",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": false
            }))
            .send()
            .await
            .expect("chat completion request");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let json: serde_json::Value = resp.json().await.expect("parse json");
        assert_eq!(json["usage"]["prompt_tokens_details"]["cached_tokens"], 200);
        assert_eq!(json["usage"]["prompt_tokens_details"]["cache_write_tokens"], 800);
        assert_eq!(json["usage"]["cache_write_tokens"], 800);
        assert_eq!(json["usage"]["cache_creation_input_tokens"], 800);
        assert_eq!(json["usage"]["completion_tokens_details"]["reasoning_tokens"], 0);

        handle.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_https_proxy_injects_include_usage_and_enriches_sse_stream() {
        let dir = std::env::temp_dir().join(format!(
            "gufo_proxy_chat_sse_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);

        let prepared = prepare_tls_server_config("", "", &dir, &[], |s| PathBuf::from(s))
            .expect("generate cert");

        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = upstream.accept().await {
                let (buf, head_end) = match read_http_head(&mut sock, Some(Duration::from_secs(5))).await {
                    HeadReadResult::Complete { buf, header_end } => (buf, header_end),
                    _ => return,
                };
                let cl = find_header_value(&buf[..head_end], "content-length")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0u8; cl];
                let mut reader = std::io::Cursor::new(&buf[head_end..]).chain(&mut sock);
                let _ = reader.read_exact(&mut body).await;
                if let Ok(req_json) = serde_json::from_slice::<serde_json::Value>(&body) {
                    let _ = seen_tx.send(req_json);
                }

                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                let _ = sock.write_all(head.as_bytes()).await;

                let sse1 = "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"}}]}\n\n";
                let sse2 = "data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":1500,\"completion_tokens\":25,\"total_tokens\":1525,\"prompt_tokens_details\":{\"cached_tokens\":300},\"completion_tokens_details\":{\"reasoning_tokens\":5},\"cached_tokens\":300,\"gufo\":{\"prefill_tokens\":1200}},\"timings\":{\"prompt_n\":1200,\"cache_n\":300}}\n\n";
                let sse3 = "data: [DONE]\n\n";

                for ev in [sse1, sse2, sse3] {
                    let _ = write_http_chunk(&mut sock, ev.as_bytes()).await;
                }
                let _ = sock.write_all(b"0\r\n\r\n").await;
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
        .expect("start proxy");
        let https_port = handle.local_addr().port();

        let cert_pem = fs::read(&prepared.cert_path).unwrap();
        let root_cert = reqwest::Certificate::from_pem(&cert_pem).unwrap();
        let client = reqwest::Client::builder()
            .add_root_certificate(root_cert)
            .build()
            .unwrap();

        // 客戶端未帶 stream_options
        let resp = client
            .post(format!("https://127.0.0.1:{}/v1/chat/completions", https_port))
            .json(&serde_json::json!({
                "model": "flash-next",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true
            }))
            .send()
            .await
            .expect("sse request");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let text = resp.text().await.expect("read sse stream");

        // 驗證上游確實收到自動注入的 stream_options.include_usage: true
        let upstream_req = seen_rx.await.expect("upstream received request");
        assert_eq!(upstream_req["stream_options"]["include_usage"], serde_json::json!(true));

        // 驗證客戶端收到的 SSE usage chunk 已補齊 cache_write_tokens 與 cache_creation_input_tokens
        let usage_chunk: serde_json::Value = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter(|p| *p != "[DONE]")
            .filter_map(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .find(|v| v.get("usage").is_some())
            .expect("should contain usage chunk");

        assert_eq!(usage_chunk["usage"]["prompt_tokens_details"]["cached_tokens"], 300);
        assert_eq!(usage_chunk["usage"]["prompt_tokens_details"]["cache_write_tokens"], 1200);
        assert_eq!(usage_chunk["usage"]["completion_tokens_details"]["reasoning_tokens"], 5);
        assert_eq!(usage_chunk["usage"]["cache_write_tokens"], 1200);
        assert_eq!(usage_chunk["usage"]["cache_creation_input_tokens"], 1200);

        handle.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }
}
