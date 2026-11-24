use reqwest::header::{ACCEPT, ACCEPT_ENCODING, ACCEPT_LANGUAGE, USER_AGENT};
use reqwest::{Client, redirect};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinSet;

use crate::gufo_manager::GufoManager;

const DEFAULT_MAX_RESULTS: u32 = 5;
const DEFAULT_FETCH_TOP_N: u32 = 3;
const DEFAULT_MAX_PAGE_CHARS: u32 = 1500;
const DEFAULT_TIMEOUT_SECS: u32 = 10;
const MAX_FETCH_BYTES: usize = 512 * 1024;
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub const DEFAULT_WEB_PROMPT_TEMPLATE: &str = "目前日期為 {{date}}。請優先依據以下「網路搜尋結果」中的最新資訊回答使用者的問題。回答時請盡量附上引用來源編號（例如 [W1]、[W2]）與重點事實；若搜尋結果無法涵蓋問題或彼此矛盾，請誠實說明，不要編造事實。\n\n【網路搜尋結果】\n{{results}}";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchConfig {
    /// "auto" | "duckduckgo" | "bing_rss" | "searxng" | "tavily" | "brave" | "serper"
    pub provider: String,
    pub api_key: String,
    pub searxng_url: String,
    pub max_results: u32,
    pub fetch_page_content: bool,
    pub fetch_top_n: u32,
    pub max_page_chars: u32,
    pub timeout_secs: u32,
    pub region: String,
    pub prompt_template: String,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: "auto".to_string(),
            api_key: String::new(),
            searxng_url: "http://127.0.0.1:8080".to_string(),
            max_results: DEFAULT_MAX_RESULTS,
            fetch_page_content: true,
            fetch_top_n: DEFAULT_FETCH_TOP_N,
            max_page_chars: DEFAULT_MAX_PAGE_CHARS,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            region: String::new(),
            prompt_template: DEFAULT_WEB_PROMPT_TEMPLATE.to_string(),
        }
    }
}

impl WebSearchConfig {
    pub fn sanitized(mut self) -> Self {
        let p = self.provider.trim().to_ascii_lowercase();
        self.provider = match p.as_str() {
            "auto" | "duckduckgo" | "bing_rss" | "searxng" | "tavily" | "brave" | "serper" => p,
            _ => "auto".to_string(),
        };
        self.api_key = self.api_key.trim().to_string();
        self.searxng_url = self.searxng_url.trim().trim_end_matches('/').to_string();
        if self.searxng_url.is_empty() {
            self.searxng_url = "http://127.0.0.1:8080".to_string();
        }
        self.max_results = self.max_results.clamp(1, 10);
        self.fetch_top_n = self.fetch_top_n.clamp(1, 5);
        self.max_page_chars = self.max_page_chars.clamp(300, 6000);
        self.timeout_secs = self.timeout_secs.clamp(3, 30);
        self.region = self.region.trim().to_string();
        if self.prompt_template.trim().is_empty() {
            self.prompt_template = DEFAULT_WEB_PROMPT_TEMPLATE.to_string();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchHit {
    pub title: String,
    pub url: String,
    pub domain: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub query: String,
    pub provider_used: String,
    pub hits: Vec<WebSearchHit>,
    pub warning: Option<String>,
    pub elapsed_ms: u64,
}

pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            b => {
                out.push('%');
                out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0').to_ascii_uppercase());
                out.push(char::from_digit((b & 0x0F) as u32, 16).unwrap_or('0').to_ascii_uppercase());
            }
        }
    }
    out
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push(((h << 4) | l) as u8);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx + 1..];
        if let Some(semi_idx) = after_amp.find(';')
            && semi_idx <= 12
        {
            let entity = &after_amp[..semi_idx];
            let decoded: Option<char> = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "ndash" => Some('–'),
                "mdash" => Some('—'),
                "hellip" => Some('…'),
                "lsquo" => Some('‘'),
                "rsquo" => Some('’'),
                "ldquo" => Some('“'),
                "rdquo" => Some('”'),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
                }
                _ if entity.starts_with('#') => {
                    entity[1..].parse::<u32>().ok().and_then(char::from_u32)
                }
                _ => None,
            };
            if let Some(ch) = decoded {
                out.push(ch);
                rest = &after_amp[semi_idx + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

pub fn extract_domain(url: &str) -> String {
    if let Ok(parsed) = reqwest::Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        return host.trim_start_matches("www.").to_string();
    }
    String::new()
}

pub fn unwrap_ddg_redirect(href: &str) -> String {
    let decoded_href = decode_html_entities(href.trim());
    let normalized = if decoded_href.starts_with("//") {
        format!("https:{}", decoded_href)
    } else if decoded_href.starts_with('/') {
        format!("https://duckduckgo.com{}", decoded_href)
    } else {
        decoded_href
    };

    if let Some(query_start) = normalized.find('?') {
        let query = &normalized[query_start + 1..];
        for pair in query.split('&') {
            if let Some(val) = pair.strip_prefix("uddg=") {
                let target = percent_decode(val);
                if target.starts_with("http://") || target.starts_with("https://") {
                    return target;
                }
            }
        }
    }
    normalized
}

pub fn is_ddg_challenge_page(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("anomaly-modal")
        || lower.contains("bots use duckduckgo")
        || lower.contains("cc=botnet")
        || (lower.contains("captcha") && lower.contains("duckduckgo"))
}

fn strip_all_tags(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn collapse_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_attr(attrs: &str, attr_name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let target = format!("{}=", attr_name.to_ascii_lowercase());
    let mut search_from = 0;
    while let Some(rel_idx) = lower[search_from..].find(&target) {
        let idx = search_from + rel_idx;
        if idx > 0 {
            let prev = lower.as_bytes()[idx - 1];
            if !prev.is_ascii_whitespace() {
                search_from = idx + target.len();
                continue;
            }
        }
        let val_start = idx + target.len();
        let rest = &attrs[val_start..];
        let trimmed = rest.trim_start();
        if let Some(after_quote) = trimmed.strip_prefix('"')
            && let Some(end) = after_quote.find('"')
        {
            return Some(after_quote[..end].to_string());
        }
        if let Some(after_quote) = trimmed.strip_prefix('\'')
            && let Some(end) = after_quote.find('\'')
        {
            return Some(after_quote[..end].to_string());
        }
        let end = trimmed
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(trimmed.len());
        return Some(trimmed[..end].to_string());
    }
    None
}

fn has_class(attrs: &str, target_class: &str) -> bool {
    extract_attr(attrs, "class")
        .map(|cls| cls.split_whitespace().any(|c| c == target_class))
        .unwrap_or(false)
}

pub fn parse_ddg_html(html: &str, max_results: usize) -> Result<Vec<WebSearchHit>, String> {
    if is_ddg_challenge_page(html) {
        return Err("DuckDuckGo 要求進行反機器人驗證 (Challenge/CAPTCHA)，無法直接擷取結果".to_string());
    }

    let lower = html.to_ascii_lowercase();
    let mut hits = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut pos = 0usize;

    while hits.len() < max_results {
        let Some(rel_a) = lower[pos..].find("<a ") else {
            break;
        };
        let a_start = pos + rel_a;
        let Some(rel_gt) = html[a_start..].find('>') else {
            break;
        };
        let tag_end = a_start + rel_gt;
        let attrs = &html[a_start + 3..tag_end];
        let Some(rel_close) = lower[tag_end + 1..].find("</a>") else {
            pos = tag_end + 1;
            continue;
        };
        let inner_end = tag_end + 1 + rel_close;
        pos = inner_end + 4;

        if !has_class(attrs, "result__a") {
            continue;
        }
        let Some(href) = extract_attr(attrs, "href") else {
            continue;
        };
        let target_url = unwrap_ddg_redirect(&href);
        if (!target_url.starts_with("http://") && !target_url.starts_with("https://"))
            || target_url.contains("duckduckgo.com/y.js")
        {
            continue;
        }

        let title = collapse_whitespace(&decode_html_entities(&strip_all_tags(
            &html[tag_end + 1..inner_end],
        )));
        if title.is_empty() || !seen_urls.insert(target_url.clone()) {
            continue;
        }

        // 在接下來的區塊中尋找對應的 result__snippet（截斷至下一個 result__a 之前）
        let lookahead_end = lower[pos..]
            .find("result__a")
            .map(|off| pos + off)
            .unwrap_or_else(|| (pos + 4000).min(html.len()));
        let window = &html[pos..lookahead_end];
        let window_lower = &lower[pos..lookahead_end];
        let mut snippet = String::new();

        if let Some(snip_idx) = window_lower.find("result__snippet")
            && let Some(gt_rel) = window[snip_idx..].find('>')
        {
            let content_start = snip_idx + gt_rel + 1;
            let close_rel = window_lower[content_start..]
                .find("</a>")
                .or_else(|| window_lower[content_start..].find("</div"))
                .or_else(|| window_lower[content_start..].find("</span"));
            if let Some(end_rel) = close_rel {
                snippet = collapse_whitespace(&decode_html_entities(&strip_all_tags(
                    &window[content_start..content_start + end_rel],
                )));
            }
        }

        hits.push(WebSearchHit {
            domain: extract_domain(&target_url),
            title,
            url: target_url,
            snippet,
            fetched_content: None,
            published_date: None,
        });
    }

    Ok(hits)
}

fn extract_xml_tag(block: &str, tag: &str) -> Option<String> {
    let lower = block.to_ascii_lowercase();
    let open_pat = format!("<{}", tag.to_ascii_lowercase());
    let close_pat = format!("</{}>", tag.to_ascii_lowercase());

    let open_start = lower.find(&open_pat)?;
    let after_open = &block[open_start + open_pat.len()..];
    let gt_idx = after_open.find('>')?;
    let content_start = open_start + open_pat.len() + gt_idx + 1;
    let close_rel = lower[content_start..].find(&close_pat)?;
    let raw = block[content_start..content_start + close_rel].trim();
    let unwrapped = raw
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(raw)
        .trim();
    Some(decode_html_entities(unwrapped))
}

pub fn parse_bing_rss(xml: &str, max_results: usize) -> Vec<WebSearchHit> {
    let lower = xml.to_ascii_lowercase();
    let mut hits = Vec::new();
    let mut seen_urls = HashSet::new();
    let mut pos = 0usize;

    while hits.len() < max_results {
        let Some(item_rel) = lower[pos..].find("<item>") else {
            break;
        };
        let item_start = pos + item_rel + 6;
        let Some(end_rel) = lower[item_start..].find("</item>") else {
            break;
        };
        let item_body = &xml[item_start..item_start + end_rel];
        pos = item_start + end_rel + 7;

        let Some(title_raw) = extract_xml_tag(item_body, "title") else {
            continue;
        };
        let Some(url_raw) = extract_xml_tag(item_body, "link") else {
            continue;
        };
        let url = url_raw.trim().to_string();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            continue;
        }
        let title = collapse_whitespace(&strip_all_tags(&title_raw));
        if title.is_empty() || !seen_urls.insert(url.clone()) {
            continue;
        }

        let snippet = extract_xml_tag(item_body, "description")
            .map(|d| collapse_whitespace(&strip_all_tags(&d)))
            .unwrap_or_default();
        let published_date = extract_xml_tag(item_body, "pubDate")
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());

        hits.push(WebSearchHit {
            domain: extract_domain(&url),
            title,
            url,
            snippet,
            fetched_content: None,
            published_date,
        });
    }

    hits
}

fn strip_ignored_blocks(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut out = String::with_capacity(html.len());
    let mut pos = 0usize;

    while pos < html.len() {
        let Some(rel_start) = lower[pos..].find(&open_pat) else {
            out.push_str(&html[pos..]);
            break;
        };
        let abs_start = pos + rel_start;
        let after_name = abs_start + open_pat.len();
        let valid_boundary = html
            .as_bytes()
            .get(after_name)
            .map(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
            .unwrap_or(false);

        if !valid_boundary {
            out.push_str(&html[pos..after_name]);
            pos = after_name;
            continue;
        }

        out.push_str(&html[pos..abs_start]);
        if let Some(rel_close) = lower[after_name..].find(&close_pat) {
            pos = after_name + rel_close + close_pat.len();
        } else if let Some(rel_gt) = html[after_name..].find('>') {
            pos = after_name + rel_gt + 1;
        } else {
            break;
        }
    }
    out
}

pub fn extract_readable_text(html: &str, max_chars: usize) -> String {
    let mut cleaned = html.to_string();
    for tag in [
        "script", "style", "noscript", "svg", "iframe", "nav", "footer", "header",
    ] {
        cleaned = strip_ignored_blocks(&cleaned, tag);
    }

    let mut text = String::with_capacity(cleaned.len() / 2);
    let mut pos = 0usize;
    let bytes = cleaned.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'<'
            && let Some(gt_rel) = cleaned[pos..].find('>')
        {
            let tag_inner = cleaned[pos + 1..pos + gt_rel].trim();
            let tag_name = tag_inner
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_closing = tag_inner.starts_with('/');
            if tag_name == "br"
                || tag_name == "hr"
                || (is_closing
                    && matches!(
                        tag_name.as_str(),
                        "p" | "div"
                            | "li"
                            | "tr"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "article"
                            | "section"
                            | "blockquote"
                            | "pre"
                    ))
            {
                text.push('\n');
            } else {
                text.push(' ');
            }
            pos += gt_rel + 1;
            continue;
        }

        let Some(next_lt) = cleaned[pos..].find('<') else {
            text.push_str(&cleaned[pos..]);
            break;
        };
        text.push_str(&cleaned[pos..pos + next_lt]);
        pos += next_lt;
    }

    let decoded = decode_html_entities(&text);
    let mut lines = Vec::new();
    for raw_line in decoded.lines() {
        let collapsed = collapse_whitespace(raw_line);
        if !collapsed.is_empty() {
            lines.push(collapsed);
        }
    }

    let joined = lines.join("\n");
    let total_chars = joined.chars().count();
    if total_chars <= max_chars {
        return joined;
    }
    let truncated: String = joined.chars().take(max_chars).collect();
    format!("{}...", truncated.trim_end())
}

fn is_public_ipv4(v4: Ipv4Addr) -> bool {
    if v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
        || v4.is_multicast()
    {
        return false;
    }
    let oct = v4.octets();
    // CGNAT 100.64.0.0/10
    if oct[0] == 100 && (oct[1] & 0b1100_0000) == 64 {
        return false;
    }
    // 192.0.0.0/24 (IETF protocol assignments)
    if oct[0] == 192 && oct[1] == 0 && oct[2] == 0 {
        return false;
    }
    // 198.18.0.0/15 (benchmarking)
    if oct[0] == 198 && (oct[1] == 18 || oct[1] == 19) {
        return false;
    }
    // 240.0.0.0/4 (reserved)
    if oct[0] >= 240 {
        return false;
    }
    true
}

fn is_public_ipv6(v6: Ipv6Addr) -> bool {
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return false;
    }
    let seg = v6.segments();
    // Unique Local fc00::/7
    if (seg[0] & 0xfe00) == 0xfc00 {
        return false;
    }
    // Link-Local fe80::/10
    if (seg[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // Documentation 2001:db8::/32
    if seg[0] == 0x2001 && seg[1] == 0x0db8 {
        return false;
    }
    true
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => is_public_ipv6(v6),
    }
}

pub fn is_safe_public_url(url_str: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url_str) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(raw_host) = parsed.host_str() else {
        return false;
    };
    let host = raw_host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();

    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".localdomain")
    {
        return false;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_ip(ip);
    }

    // 拒絕無網域後綴的單標籤內網主機名稱 (如 http://metadata/)
    host.contains('.')
}

async fn host_resolves_to_public_ips(url_str: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url_str) else {
        return false;
    };
    let Some(raw_host) = parsed.host_str() else {
        return false;
    };
    let host = raw_host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_ip(ip);
    }
    let port = parsed.port_or_known_default().unwrap_or(443);
    let Ok(addrs) = tokio::net::lookup_host((host, port)).await else {
        return false;
    };
    let mut count = 0usize;
    for addr in addrs {
        count += 1;
        if !is_public_ip(addr.ip()) {
            return false;
        }
    }
    count > 0
}

fn build_http_client(timeout_secs: u32, check_redirects: bool) -> Client {
    let mut builder = Client::builder().timeout(Duration::from_secs(timeout_secs as u64));
    if check_redirects {
        builder = builder.redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.stop()
            } else if !is_safe_public_url(attempt.url().as_str()) {
                attempt.error("已攔截導向至非公開或無效網址的重定向")
            } else {
                attempt.follow()
            }
        }));
    }
    builder.build().unwrap_or_default()
}

async fn search_duckduckgo(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    let mut url = format!("https://html.duckduckgo.com/html/?q={}", percent_encode(query));
    if !cfg.region.is_empty() {
        url.push_str(&format!("&kl={}", percent_encode(&cfg.region)));
    }

    let resp = client
        .get(&url)
        .header(USER_AGENT, BROWSER_USER_AGENT)
        .header(ACCEPT_ENCODING, "identity")
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(ACCEPT_LANGUAGE, "zh-TW,zh;q=0.9,en;q=0.8")
        .send()
        .await
        .map_err(|e| format!("連線 DuckDuckGo 失敗：{}", e))?;

    let status = resp.status();
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Err(format!(
            "DuckDuckGo 回應 HTTP {}（觸發頻率限制或驗證要求）",
            status
        ));
    }
    if !status.is_success() {
        return Err(format!("DuckDuckGo 回應 HTTP {}", status));
    }

    let html = resp
        .text()
        .await
        .map_err(|e| format!("讀取 DuckDuckGo 回應失敗：{}", e))?;
    parse_ddg_html(&html, cfg.max_results as usize)
}

async fn search_bing_rss(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    let mut url = format!(
        "https://www.bing.com/search?q={}&format=rss&count={}",
        percent_encode(query),
        cfg.max_results
    );
    if !cfg.region.is_empty() {
        url.push_str(&format!("&setlang={}", percent_encode(&cfg.region)));
    }

    let resp = client
        .get(&url)
        .header(USER_AGENT, BROWSER_USER_AGENT)
        .header(ACCEPT_ENCODING, "identity")
        .header(ACCEPT, "application/rss+xml,application/xml,text/xml")
        .send()
        .await
        .map_err(|e| format!("連線 Bing RSS 失敗：{}", e))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Bing RSS 回應 HTTP {}", status));
    }

    let xml = resp
        .text()
        .await
        .map_err(|e| format!("讀取 Bing RSS 回應失敗：{}", e))?;
    Ok(parse_bing_rss(&xml, cfg.max_results as usize))
}

#[derive(Deserialize)]
struct SearxngResponse {
    #[serde(default)]
    results: Vec<SearxngItem>,
}

#[derive(Deserialize)]
struct SearxngItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
}

async fn search_searxng(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    let base = cfg.searxng_url.trim().trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err("SearXNG 網址必須以 http:// 或 https:// 開頭".to_string());
    }
    let mut url = format!("{}/search?q={}&format=json&pageno=1", base, percent_encode(query));
    if !cfg.region.is_empty() {
        url.push_str(&format!("&language={}", percent_encode(&cfg.region)));
    }

    let resp = client
        .get(&url)
        .header(USER_AGENT, BROWSER_USER_AGENT)
        .header(ACCEPT_ENCODING, "identity")
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("連線 SearXNG 失敗 ({}): {}", base, e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("讀取 SearXNG 回應失敗：{}", e))?;
    if !status.is_success() {
        let preview: String = body.chars().take(240).collect();
        return Err(format!("SearXNG 回應 HTTP {}：{}", status, preview));
    }

    let parsed: SearxngResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析 SearXNG JSON 失敗：{}", e))?;

    let mut hits = Vec::new();
    for item in parsed.results.into_iter().take(cfg.max_results as usize) {
        if item.url.trim().is_empty() || item.title.trim().is_empty() {
            continue;
        }
        hits.push(WebSearchHit {
            domain: extract_domain(&item.url),
            title: collapse_whitespace(&item.title),
            url: item.url,
            snippet: collapse_whitespace(&strip_all_tags(&item.content)),
            fetched_content: None,
            published_date: item.published_date.filter(|d| !d.trim().is_empty()),
        });
    }
    Ok(hits)
}

#[derive(Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyItem>,
}

#[derive(Deserialize)]
struct TavilyItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    published_date: Option<String>,
}

async fn search_tavily(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("尚未設定 Tavily API Key".to_string());
    }

    let resp = client
        .post("https://api.tavily.com/search")
        .header(ACCEPT_ENCODING, "identity")
        .json(&serde_json::json!({
            "api_key": cfg.api_key,
            "query": query,
            "max_results": cfg.max_results,
            "search_depth": "basic",
            "include_answer": false
        }))
        .send()
        .await
        .map_err(|e| format!("連線 Tavily 失敗：{}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("讀取 Tavily 回應失敗：{}", e))?;
    if !status.is_success() {
        let preview: String = body.chars().take(240).collect();
        return Err(format!("Tavily API 回傳 HTTP {}：{}", status, preview));
    }

    let parsed: TavilyResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析 Tavily JSON 失敗：{}", e))?;

    Ok(parsed
        .results
        .into_iter()
        .take(cfg.max_results as usize)
        .filter(|item| !item.url.trim().is_empty())
        .map(|item| WebSearchHit {
            domain: extract_domain(&item.url),
            title: collapse_whitespace(&item.title),
            url: item.url,
            snippet: collapse_whitespace(&item.content),
            fetched_content: None,
            published_date: item.published_date.filter(|d| !d.trim().is_empty()),
        })
        .collect())
}

#[derive(Deserialize)]
struct BraveResponse {
    web: Option<BraveWebBlock>,
}

#[derive(Deserialize)]
struct BraveWebBlock {
    #[serde(default)]
    results: Vec<BraveItem>,
}

#[derive(Deserialize)]
struct BraveItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    page_age: Option<String>,
}

async fn search_brave(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("尚未設定 Brave Search API Key".to_string());
    }
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        percent_encode(query),
        cfg.max_results
    );

    let resp = client
        .get(&url)
        .header("X-Subscription-Token", &cfg.api_key)
        .header(ACCEPT, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|e| format!("連線 Brave Search 失敗：{}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("讀取 Brave Search 回應失敗：{}", e))?;
    if !status.is_success() {
        let preview: String = body.chars().take(240).collect();
        return Err(format!("Brave API 回傳 HTTP {}：{}", status, preview));
    }

    let parsed: BraveResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析 Brave JSON 失敗：{}", e))?;

    Ok(parsed
        .web
        .map(|w| w.results)
        .unwrap_or_default()
        .into_iter()
        .take(cfg.max_results as usize)
        .filter(|item| !item.url.trim().is_empty())
        .map(|item| WebSearchHit {
            domain: extract_domain(&item.url),
            title: collapse_whitespace(&item.title),
            url: item.url,
            snippet: collapse_whitespace(&strip_all_tags(&item.description)),
            fetched_content: None,
            published_date: item.page_age.filter(|d| !d.trim().is_empty()),
        })
        .collect())
}

#[derive(Deserialize)]
struct SerperResponse {
    #[serde(default)]
    organic: Vec<SerperItem>,
}

#[derive(Deserialize)]
struct SerperItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    link: String,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    date: Option<String>,
}

async fn search_serper(
    client: &Client,
    query: &str,
    cfg: &WebSearchConfig,
) -> Result<Vec<WebSearchHit>, String> {
    if cfg.api_key.trim().is_empty() {
        return Err("尚未設定 Serper API Key".to_string());
    }

    let mut payload = serde_json::json!({
        "q": query,
        "num": cfg.max_results
    });
    if !cfg.region.is_empty() {
        payload["gl"] = serde_json::Value::String(cfg.region.clone());
    }

    let resp = client
        .post("https://google.serper.dev/search")
        .header("X-API-KEY", &cfg.api_key)
        .header(ACCEPT_ENCODING, "identity")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("連線 Serper 失敗：{}", e))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("讀取 Serper 回應失敗：{}", e))?;
    if !status.is_success() {
        let preview: String = body.chars().take(240).collect();
        return Err(format!("Serper API 回傳 HTTP {}：{}", status, preview));
    }

    let parsed: SerperResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析 Serper JSON 失敗：{}", e))?;

    Ok(parsed
        .organic
        .into_iter()
        .take(cfg.max_results as usize)
        .filter(|item| !item.link.trim().is_empty())
        .map(|item| WebSearchHit {
            domain: extract_domain(&item.link),
            title: collapse_whitespace(&item.title),
            url: item.link,
            snippet: collapse_whitespace(&item.snippet),
            fetched_content: None,
            published_date: item.date.filter(|d| !d.trim().is_empty()),
        })
        .collect())
}

async fn fetch_single_page(client: &Client, url: &str, max_chars: usize) -> Option<String> {
    if !is_safe_public_url(url) || !host_resolves_to_public_ips(url).await {
        return None;
    }

    let mut resp = client
        .get(url)
        .header(USER_AGENT, BROWSER_USER_AGENT)
        .header(ACCEPT_ENCODING, "identity")
        .header(ACCEPT, "text/html,text/plain;q=0.9,*/*;q=0.5")
        .header(ACCEPT_LANGUAGE, "zh-TW,zh;q=0.9,en;q=0.8")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() || !is_safe_public_url(resp.url().as_str()) {
        return None;
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let is_plain = content_type.contains("text/plain");
    let is_html = content_type.is_empty()
        || content_type.contains("text/html")
        || content_type.contains("application/xhtml+xml");
    if !is_plain && !is_html {
        return None;
    }

    let mut buf = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        let remaining = MAX_FETCH_BYTES.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let take = chunk.len().min(remaining);
        buf.extend_from_slice(&chunk[..take]);
        if buf.len() >= MAX_FETCH_BYTES {
            break;
        }
    }

    if buf.is_empty() {
        return None;
    }
    let body = String::from_utf8_lossy(&buf);
    let extracted = if is_plain {
        let trimmed = body.trim();
        if trimmed.chars().count() <= max_chars {
            trimmed.to_string()
        } else {
            let cut: String = trimmed.chars().take(max_chars).collect();
            format!("{}...", cut.trim_end())
        }
    } else {
        extract_readable_text(&body, max_chars)
    };

    if extracted.trim().chars().count() < 40 {
        None
    } else {
        Some(extracted)
    }
}

async fn enrich_hits_with_pages(hits: &mut [WebSearchHit], cfg: &WebSearchConfig) {
    let limit = (cfg.fetch_top_n as usize).min(hits.len());
    if limit == 0 {
        return;
    }

    let fetch_client = build_http_client(cfg.timeout_secs.min(8), true);
    let max_chars = cfg.max_page_chars as usize;
    let mut join_set = JoinSet::new();

    for (idx, hit) in hits.iter().take(limit).enumerate() {
        let client = fetch_client.clone();
        let url = hit.url.clone();
        join_set.spawn(async move {
            let text = fetch_single_page(&client, &url, max_chars).await;
            (idx, text)
        });
    }

    while let Some(res) = join_set.join_next().await {
        if let Ok((idx, Some(content))) = res
            && let Some(hit) = hits.get_mut(idx)
        {
            hit.fetched_content = Some(content);
        }
    }
}

pub struct WebSearchManager {
    config_path: PathBuf,
    config: RwLock<WebSearchConfig>,
}

impl Default for WebSearchManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchManager {
    pub fn new() -> Self {
        Self::with_config_path(GufoManager::app_base_dir().join("gufo-websearch.json"))
    }

    pub fn with_config_path(config_path: PathBuf) -> Self {
        let config = Self::load_config_from_disk(&config_path)
            .unwrap_or_default()
            .sanitized();
        Self {
            config_path,
            config: RwLock::new(config),
        }
    }

    fn load_config_from_disk(path: &Path) -> Option<WebSearchConfig> {
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<WebSearchConfig>(&raw).ok()
    }

    fn write_config_atomic(&self, cfg: &WebSearchConfig) -> Result<(), String> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("建立目錄失敗：{}", e))?;
        }
        let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
        let tmp_path = self.config_path.with_extension("json.tmp");
        fs::write(&tmp_path, json)
            .map_err(|e| format!("寫入暫存設定檔失敗 {}: {}", tmp_path.display(), e))?;
        if let Err(err) = fs::rename(&tmp_path, &self.config_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!(
                "儲存 WebSearch 設定檔失敗 {}: {}",
                self.config_path.display(),
                err
            ));
        }
        Ok(())
    }

    pub async fn get_config(&self) -> WebSearchConfig {
        self.config.read().await.clone()
    }

    pub async fn save_config(&self, cfg: WebSearchConfig) -> Result<WebSearchConfig, String> {
        let clean = cfg.sanitized();
        self.write_config_atomic(&clean)?;
        *self.config.write().await = clean.clone();
        Ok(clean)
    }

    pub async fn search(
        &self,
        query: String,
        config_override: Option<WebSearchConfig>,
    ) -> Result<WebSearchResponse, String> {
        let start = Instant::now();
        let trimmed_query = query.trim().to_string();
        if trimmed_query.is_empty() {
            return Err("搜尋關鍵字不可為空".to_string());
        }

        let cfg = match config_override {
            Some(c) => c.sanitized(),
            None => self.get_config().await,
        };

        let search_client = build_http_client(cfg.timeout_secs, false);
        let mut warning: Option<String> = None;
        let provider_used: String;

        let mut hits = match cfg.provider.as_str() {
            "duckduckgo" => {
                provider_used = "duckduckgo".to_string();
                search_duckduckgo(&search_client, &trimmed_query, &cfg).await?
            }
            "bing_rss" => {
                provider_used = "bing_rss".to_string();
                search_bing_rss(&search_client, &trimmed_query, &cfg).await?
            }
            "searxng" => {
                provider_used = "searxng".to_string();
                search_searxng(&search_client, &trimmed_query, &cfg).await?
            }
            "tavily" => {
                provider_used = "tavily".to_string();
                search_tavily(&search_client, &trimmed_query, &cfg).await?
            }
            "brave" => {
                provider_used = "brave".to_string();
                search_brave(&search_client, &trimmed_query, &cfg).await?
            }
            "serper" => {
                provider_used = "serper".to_string();
                search_serper(&search_client, &trimmed_query, &cfg).await?
            }
            _ => {
                // "auto": 優先 DuckDuckGo，若失敗或無結果則自動切換至 Bing RSS
                match search_duckduckgo(&search_client, &trimmed_query, &cfg).await {
                    Ok(res) if !res.is_empty() => {
                        provider_used = "duckduckgo".to_string();
                        res
                    }
                    Ok(_) => {
                        provider_used = "bing_rss".to_string();
                        warning = Some("DuckDuckGo 無回傳結果，已自動切換為 Bing RSS".to_string());
                        search_bing_rss(&search_client, &trimmed_query, &cfg).await?
                    }
                    Err(ddg_err) => {
                        provider_used = "bing_rss".to_string();
                        warning = Some(format!(
                            "DuckDuckGo 無法使用（{}），已自動切換為 Bing RSS",
                            ddg_err
                        ));
                        search_bing_rss(&search_client, &trimmed_query, &cfg).await?
                    }
                }
            }
        };

        if cfg.fetch_page_content && !hits.is_empty() {
            enrich_hits_with_pages(&mut hits, &cfg).await;
        }

        Ok(WebSearchResponse {
            query: trimmed_query,
            provider_used,
            hits,
            warning,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn test_percent_encode_and_ddg_redirect_unwrap() {
        let encoded = percent_encode("Gufo Studio 測試");
        assert_eq!(encoded, "Gufo+Studio+%E6%B8%AC%E8%A9%A6");
        assert_eq!(percent_decode(&encoded), "Gufo Studio 測試");

        let ddg_href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs%3Fa%3D1%26b%3D2&rut=abc";
        assert_eq!(
            unwrap_ddg_redirect(ddg_href),
            "https://example.com/docs?a=1&b=2"
        );
    }

    #[test]
    fn test_parse_ddg_html_and_challenge_detection() {
        let sample = r#"
        <div class="result results_links">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F">The <b>Rust</b> Programming Language</a>
          <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F">A language empowering everyone to build &amp; ship reliable software.</a>
        </div>
        <div class="result results_links">
          <a class="result__a" href="https://tauri.app/">Tauri 2.0</a>
          <div class="result__snippet">Build smaller, faster, and more secure desktop apps.</div>
        </div>
        "#;

        let hits = parse_ddg_html(sample, 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert_eq!(hits[0].domain, "rust-lang.org");
        assert_eq!(
            hits[0].snippet,
            "A language empowering everyone to build & ship reliable software."
        );
        assert_eq!(hits[1].title, "Tauri 2.0");

        let challenge = r#"<html><body><div class="anomaly-modal">Bots use DuckDuckGo too</div></body></html>"#;
        assert!(is_ddg_challenge_page(challenge));
        assert!(parse_ddg_html(challenge, 5).is_err());
    }

    #[test]
    fn test_parse_bing_rss_items() {
        let rss = r#"<?xml version="1.0" encoding="utf-8" ?>
        <rss version="2.0"><channel>
          <item>
            <title><![CDATA[AMD Strix Halo & ROCm Guide]]></title>
            <link>https://www.example.org/strix-halo</link>
            <description><![CDATA[Unified memory up to <b>96GB</b> for local AI.]]></description>
            <pubDate>Tue, 24 Nov 2026 10:00:00 GMT</pubDate>
          </item>
          <item>
            <title>Gufo Release Notes</title>
            <link>https://github.com/gufo-org/gufo</link>
            <description>Fast inference &amp; embeddings.</description>
          </item>
        </channel></rss>"#;

        let hits = parse_bing_rss(rss, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "AMD Strix Halo & ROCm Guide");
        assert_eq!(hits[0].domain, "example.org");
        assert_eq!(hits[0].snippet, "Unified memory up to 96GB for local AI.");
        assert_eq!(
            hits[0].published_date.as_deref(),
            Some("Tue, 24 Nov 2026 10:00:00 GMT")
        );
        assert_eq!(hits[1].snippet, "Fast inference & embeddings.");
    }

    #[test]
    fn test_extract_readable_text_strips_noise_and_decodes_entities() {
        let html = r#"
        <html>
          <head>
            <style>.hidden { display: none; }</style>
            <script>console.log("ignore me");</script>
          </head>
          <body>
            <header><nav>Menu Home About</nav></header>
            <main>
              <h1>Gufo Studio WebSearch</h1>
              <p>支援 <b>即時搜尋</b> &amp; 網頁摘要擷取。</p>
              <p>第二段說明文字&#x21;</p>
            </main>
            <footer>Copyright 2026</footer>
          </body>
        </html>
        "#;

        let text = extract_readable_text(html, 500);
        assert!(text.contains("Gufo Studio WebSearch"));
        assert!(text.contains("支援 即時搜尋 & 網頁摘要擷取。"));
        assert!(text.contains("第二段說明文字!"));
        assert!(!text.contains("ignore me"));
        assert!(!text.contains("Menu Home"));
        assert!(!text.contains("Copyright 2026"));

        let short = extract_readable_text(html, 12);
        assert!(short.ends_with("..."));
    }

    #[test]
    fn test_ssrf_public_url_guard() {
        assert!(is_safe_public_url("https://example.com/article?id=1"));
        assert!(is_safe_public_url("http://8.8.8.8/test"));
        assert!(!is_safe_public_url("file:///C:/Windows/win.ini"));
        assert!(!is_safe_public_url("ftp://example.com/file"));
        assert!(!is_safe_public_url("http://localhost:8080/admin"));
        assert!(!is_safe_public_url("http://127.0.0.1:12223/v1"));
        assert!(!is_safe_public_url("http://192.168.1.1/"));
        assert!(!is_safe_public_url("http://10.0.0.5/"));
        assert!(!is_safe_public_url("http://169.254.169.254/latest/meta-data/"));
        assert!(!is_safe_public_url("http://[::1]:8080/"));
        assert!(!is_safe_public_url("http://[::ffff:127.0.0.1]/"));
        assert!(!is_safe_public_url("http://intranet/secret"));
    }

    #[tokio::test]
    async fn test_config_persistence_and_mock_searxng_search() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let body = serde_json::json!({
                    "results": [
                        {
                            "title": "Gufo Official Repo",
                            "url": "https://github.com/gufo-org/gufo",
                            "content": "High-performance local inference engine.",
                            "publishedDate": "2026-11-20"
                        }
                    ]
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let temp_dir = std::env::temp_dir().join(format!(
            "gufo_websearch_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let cfg_path = temp_dir.join("gufo-websearch.json");

        let mgr = WebSearchManager::with_config_path(cfg_path.clone());
        let saved = mgr
            .save_config(WebSearchConfig {
                provider: "searxng".to_string(),
                searxng_url: format!("http://{}", addr),
                max_results: 3,
                fetch_page_content: false,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(saved.provider, "searxng");

        // 從磁碟重新載入驗證持久化
        let mgr_reloaded = WebSearchManager::with_config_path(cfg_path.clone());
        let loaded_cfg = mgr_reloaded.get_config().await;
        assert_eq!(loaded_cfg.provider, "searxng");
        assert_eq!(loaded_cfg.max_results, 3);

        let res = mgr_reloaded
            .search("gufo inference".to_string(), None)
            .await
            .unwrap();
        assert_eq!(res.provider_used, "searxng");
        assert_eq!(res.hits.len(), 1);
        assert_eq!(res.hits[0].title, "Gufo Official Repo");
        assert_eq!(res.hits[0].domain, "github.com");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
