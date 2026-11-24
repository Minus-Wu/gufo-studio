use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::https_proxy::{
    self, HttpsProxyHandle, connect_host, detect_local_ipv4, effective_bind_host, format_url_host,
    is_wildcard_host,
};
use crate::process_control;

const MAX_LOG_LINES: usize = 1000;
const MAX_METRICS: usize = 50;
/// 正常關閉 (Ctrl+C / SIGINT) 的等待上限；Gufo 會在此期間排空磁碟快取寫入。
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// 模型載入超過此時間仍未就緒時，於日誌提示可能的原因 (不會中斷載入)。
const SLOW_LOAD_HINT_AFTER: Duration = Duration::from_secs(180);

/// 可儲存的多模型設定檔項目 (方便在 Qwen3.8 / Inkling-2-Mini 等模型之間一鍵切換)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub model_path: String,
    pub mtp_path: String,
    pub dspark_path: String,
    pub vision_path: String,
    pub served_model_name: String,
    pub context: u32,
    pub sessions: u32,
    pub think: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub speculative: String,
    pub yarn_mode: String,
    pub extra_args: String,
}

impl Default for ModelProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            model_path: String::new(),
            mtp_path: String::new(),
            dspark_path: String::new(),
            vision_path: String::new(),
            served_model_name: "flash-next".to_string(),
            context: 131072,
            sessions: 2,
            think: "off".to_string(),
            temperature: 0.0,
            max_tokens: 32768,
            speculative: "off".to_string(),
            yarn_mode: "auto".to_string(),
            extra_args: String::new(),
        }
    }
}

/// 缺少的欄位一律採用 `Default`，讓舊版設定檔可直接升級。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GufoServerConfig {
    pub gufo_path: String,
    pub model_path: String,
    pub mtp_path: String,
    pub dspark_path: String,
    pub vision_path: String,
    pub served_model_name: String,
    pub host: String,
    pub port: u16,
    pub context: u32,
    pub sessions: u32,
    pub max_request_bytes: u64,
    pub max_output_bytes: u64,
    pub think: String, // "off", "on", "auto"
    pub temperature: f32,
    pub speculative: String, // "mtp", "dspark", "off"
    pub mtp_policy: String,  // "survival", "length"
    pub prompt_lookup: bool,
    /// YaRN 位置編碼擴展模式："auto" (超過 256K 自動附加)、"on" (強制開啟)、"off" (強制關閉)
    pub yarn_mode: String,
    /// 額外 CLI 啟動參數 (空白分隔，支援雙引號/單引號路徑)
    pub extra_args: String,
    /// 已儲存的多模型設定檔清單
    pub model_profiles: Vec<ModelProfile>,
    /// 目前套用的設定檔 ID
    pub active_profile_id: String,
    pub api_key: String,
    pub auto_open_gui: bool,
    pub auto_start_server: bool,
    /// 請求未指定 max_tokens 時的預設生成上限 (0 = 沿用 Gufo 預設：直到 EOS 或 context 用盡)
    pub max_tokens: u32,
    /// MTP 每輪草稿 token 數 (0 = 沿用 Gufo 預設)
    pub draft_tokens: u32,
    /// MTP 草稿詞表："full" (預設) 或 "latin" (僅英文/程式碼輸出時較快)
    pub mtp_draft_vocab: String,
    /// Prefill 分塊大小 (0 = 沿用 Gufo 預設)
    pub prefill_chunk: u32,
    /// 啟用可跨重啟保留的磁碟前綴快取 (--cache-disk)
    pub cache_disk: bool,
    pub cache_disk_dir: String,
    /// 磁碟快取容量上限 bytes (0 = 沿用 Gufo 預設)
    pub cache_disk_bytes: u64,
    /// 是否啟用內建 HTTPS (TLS) 反向代理
    pub https_enabled: bool,
    /// HTTPS 代理監聽埠號 (與 HTTP Port 分開)
    pub https_port: u16,
    /// 自訂 TLS 憑證 PEM 路徑 (留空則自動產生並重複使用自簽憑證)
    pub https_cert_path: String,
    /// 自訂 TLS 私鑰 PEM 路徑 (留空則自動產生並重複使用自簽憑證)
    pub https_key_path: String,
    /// 自動自簽憑證額外包含的 SAN (IP 或網域名稱，逗號分隔)
    pub https_sans: String,
}

impl Default for GufoServerConfig {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let exists = |rel: &Path| cwd.join(rel).exists() || exe_dir.join(rel).exists();

        // 工作目錄下尋找 gufo.exe 的相對候選
        let gufo_candidates = [r"gufo.exe", r"build\release\gufo.exe", r"bin\gufo.exe"];
        let default_gufo = gufo_candidates
            .iter()
            .find(|rel| exists(Path::new(rel)))
            .unwrap_or(&gufo_candidates[0])
            .to_string();

        // 工作目錄下尋找模型目錄的相對候選
        const MODEL_FILE: &str = "Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf";
        let model_dir_candidates = [r"models\Qwen3.8-Flash-Next-GGUF", r"models", r""];
        let chosen_model_dir = model_dir_candidates
            .iter()
            .find(|rel| exists(&Path::new(rel).join(MODEL_FILE)))
            .unwrap_or(&model_dir_candidates[0]);
        let in_model_dir = |file: &str| {
            if chosen_model_dir.is_empty() {
                file.to_string()
            } else {
                format!(r"{}\{}", chosen_model_dir, file)
            }
        };

        Self {
            gufo_path: default_gufo,
            model_path: in_model_dir(MODEL_FILE),
            mtp_path: in_model_dir("mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf"),
            dspark_path: String::new(),
            vision_path: in_model_dir("mmproj-BF16.gguf"),
            served_model_name: "flash-next".to_string(),
            host: "0.0.0.0".to_string(),
            port: 12223,
            context: 131072, // 128K context
            sessions: 2,
            max_request_bytes: 67108864, // 64MB (支援多模態圖片上傳)
            max_output_bytes: 8388608,   // 8MB
            think: "off".to_string(),
            temperature: 0.0,
            speculative: "mtp".to_string(),
            mtp_policy: "survival".to_string(),
            prompt_lookup: true,
            yarn_mode: "auto".to_string(),
            extra_args: String::new(),
            model_profiles: Vec::new(),
            active_profile_id: String::new(),
            api_key: String::new(),
            auto_open_gui: true,
            auto_start_server: false,
            max_tokens: 32768,
            draft_tokens: 0,
            mtp_draft_vocab: "full".to_string(),
            prefill_chunk: 0,
            cache_disk: true,
            cache_disk_dir: "gufo-cache".to_string(),
            cache_disk_bytes: 0,
            https_enabled: true,
            https_port: 12443,
            https_cert_path: String::new(),
            https_key_path: String::new(),
            https_sans: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServerStateEnum {
    #[default]
    Stopped,
    Starting,
    Running,
    Error,
}

impl ServerStateEnum {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
    pub state: ServerStateEnum,
    pub pid: Option<u32>,
    pub port: u16,
    pub host: String,
    pub served_model_name: String,
    pub endpoint: String,
    pub uptime_secs: u64,
    pub error_message: Option<String>,
    pub api_key: Option<String>,
    pub https_enabled: bool,
    pub https_port: u16,
    pub https_running: bool,
    pub https_endpoint: Option<String>,
    pub https_cert_path: Option<String>,
    pub lan_ip: Option<String>,
}

fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveInferenceRequest {
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub body_bytes: u64,
    pub timestamp: u64,
}

impl ActiveInferenceRequest {
    pub fn parse_received_line(line: &str) -> Option<Self> {
        if !line.contains("[http]") || !line.contains("event=received") {
            return None;
        }
        let mut req = Self {
            request_id: String::new(),
            method: String::new(),
            path: String::new(),
            body_bytes: 0,
            timestamp: now_unix_millis(),
        };
        for part in line.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                match k {
                    "request" => req.request_id = v.to_string(),
                    "method" => req.method = v.to_string(),
                    "path" => req.path = v.to_string(),
                    "body_bytes" => req.body_bytes = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        let is_llm_endpoint = matches!(
            req.path.as_str(),
            "/v1/chat/completions" | "/v1/completions" | "/v1/responses" | "/v1/messages" | "/completion"
        );
        if req.request_id.is_empty() || req.method != "POST" || !is_llm_endpoint {
            return None;
        }
        Some(req)
    }
}

pub fn parse_completed_request_id(line: &str) -> Option<String> {
    if !line.contains("[http]") || !line.contains("event=completed") {
        return None;
    }
    for part in line.split_whitespace() {
        if let Some(("request", id)) = part.split_once('=') {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceMetrics {
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: f64,
    pub prompt_tokens: u64,
    pub prefill_tokens: u64,
    pub generated_tokens: u64,
    pub ttft_ms: f64,
    pub prefill_tps: f64,
    pub decode_tps: f64,
    pub draft_accepted: u64,
    pub draft_proposed: u64,
    pub acceptance_pct: Option<f64>,
    pub outcome: String,
    pub finish: String,
    pub cache: String,
    pub cached_tokens: u64,
    pub cache_hit_pct: f64,
    pub cache_restore_ms: f64,
    pub cache_miss_reason: Option<String>,
    pub common_prefix_tokens: Option<u64>,
    pub nearest_checkpoint_tokens: Option<u64>,
    pub lookup_accepted: u64,
    pub lookup_proposed: u64,
    pub timestamp: u64,
}

impl InferenceMetrics {
    pub fn parse_log_line(line: &str) -> Option<Self> {
        if !line.contains("[http]") || (!line.contains("decode_tps=") && !line.contains("prefill_tps=")) {
            return None;
        }

        let mut metrics = Self {
            request_id: String::new(),
            method: String::new(),
            path: String::new(),
            status: 200,
            duration_ms: 0.0,
            prompt_tokens: 0,
            prefill_tokens: 0,
            generated_tokens: 0,
            ttft_ms: 0.0,
            prefill_tps: 0.0,
            decode_tps: 0.0,
            draft_accepted: 0,
            draft_proposed: 0,
            acceptance_pct: None,
            outcome: "completed".to_string(),
            finish: "stop".to_string(),
            cache: "miss".to_string(),
            cached_tokens: 0,
            cache_hit_pct: 0.0,
            cache_restore_ms: 0.0,
            cache_miss_reason: None,
            common_prefix_tokens: None,
            nearest_checkpoint_tokens: None,
            lookup_accepted: 0,
            lookup_proposed: 0,
            timestamp: now_unix_millis(),
        };

        for part in line.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                match k {
                    "request" => metrics.request_id = v.to_string(),
                    "method" => metrics.method = v.to_string(),
                    "path" => metrics.path = v.to_string(),
                    "status" => metrics.status = v.parse().unwrap_or(200),
                    "duration_ms" => metrics.duration_ms = v.parse().unwrap_or(0.0),
                    "prompt_tokens" => metrics.prompt_tokens = v.parse().unwrap_or(0),
                    "prefill_tokens" => metrics.prefill_tokens = v.parse().unwrap_or(0),
                    "generated_tokens" => metrics.generated_tokens = v.parse().unwrap_or(0),
                    "ttft_ms" => metrics.ttft_ms = v.parse().unwrap_or(0.0),
                    "prefill_tps" => metrics.prefill_tps = v.parse().unwrap_or(0.0),
                    "decode_tps" => metrics.decode_tps = v.parse().unwrap_or(0.0),
                    "draft_accepted" => metrics.draft_accepted = v.parse().unwrap_or(0),
                    "draft_proposed" => metrics.draft_proposed = v.parse().unwrap_or(0),
                    "acceptance_pct" => metrics.acceptance_pct = v.parse().ok(),
                    "outcome" => metrics.outcome = v.to_string(),
                    "finish" => metrics.finish = v.to_string(),
                    "cache" => metrics.cache = v.to_string(),
                    "cached_tokens" => metrics.cached_tokens = v.parse().unwrap_or(0),
                    "cache_restore_ms" => metrics.cache_restore_ms = v.parse().unwrap_or(0.0),
                    "cache_miss_reason" => metrics.cache_miss_reason = Some(v.to_string()),
                    "common_prefix_tokens" => metrics.common_prefix_tokens = v.parse().ok(),
                    "nearest_checkpoint_tokens" => metrics.nearest_checkpoint_tokens = v.parse().ok(),
                    "lookup_accepted" => metrics.lookup_accepted = v.parse().unwrap_or(0),
                    "lookup_proposed" => metrics.lookup_proposed = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }

        if metrics.prompt_tokens == 0 && (metrics.prefill_tokens > 0 || metrics.cached_tokens > 0) {
            metrics.prompt_tokens = metrics.prefill_tokens + metrics.cached_tokens;
        }
        if metrics.cached_tokens > 0 && metrics.cache == "miss" {
            metrics.cache = "memory".to_string();
        }
        if metrics.prompt_tokens > 0 {
            metrics.cache_hit_pct =
                ((metrics.cached_tokens as f64 * 100.0) / (metrics.prompt_tokens as f64)).min(100.0);
        }

        if metrics.draft_proposed > 0 && metrics.acceptance_pct.is_none() {
            metrics.acceptance_pct = Some(
                (metrics.draft_accepted as f64 * 100.0) / (metrics.draft_proposed as f64),
            );
        }

        Some(metrics)
    }
}

/// Gufo 在模型載入完成、HTTP 伺服器開始監聽後會輸出 `event=listening`。
/// 只比對此事件，避免 "Error starting HTTP server" 之類的錯誤訊息被誤判為就緒。
fn is_listening_line(line: &str) -> bool {
    line.contains("event=listening")
}

#[derive(Debug, Clone, Copy)]
pub enum SpeculativeArg<'a> {
    Mtp(&'a Path),
    Dspark(&'a Path),
}

/// 拆分額外 CLI 參數字串，支援單引號／雙引號包住的含空白路徑，並保留 Windows 反斜線路徑。
pub fn split_cli_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = raw.chars().peekable();

    while let Some(c) = chars.next() {
        if in_double {
            if c == '\\' && chars.peek() == Some(&'"') {
                if let Some(q) = chars.next() {
                    current.push(q);
                }
            } else if c == '"' {
                in_double = false;
            } else {
                current.push(c);
            }
        } else if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
        } else if c.is_whitespace() {
            if has_token {
                args.push(std::mem::take(&mut current));
                has_token = false;
            }
        } else if c == '"' {
            in_double = true;
            has_token = true;
        } else if c == '\'' {
            in_single = true;
            has_token = true;
        } else {
            current.push(c);
            has_token = true;
        }
    }

    if has_token {
        args.push(current);
    }
    args
}

/// 組裝官方 `gufo-org/gufo` (`v0.35.0+`) `gufo serve ... llm ...` 參數。純函式，方便測試。
pub fn build_args(
    cfg: &GufoServerConfig,
    model: &Path,
    speculative: Option<SpeculativeArg<'_>>,
    vision: Option<&Path>,
    cache_dir: Option<&Path>,
) -> Vec<String> {
    const FLASH_NEXT_NATIVE_CONTEXT: u32 = 262_144;

    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));
    let bind_host = effective_bind_host(&cfg.host);

    push(&["serve", "--host", &bind_host, "--port", &cfg.port.to_string()]);
    push(&["--sessions", &cfg.sessions.to_string()]);
    push(&["--max-request-bytes", &cfg.max_request_bytes.to_string()]);

    push(&["llm", "--model", &model.to_string_lossy()]);
    push(&["--served-model-name", &cfg.served_model_name]);
    push(&["--context", &cfg.context.to_string()]);
    let enable_yarn = match cfg.yarn_mode.trim().to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => cfg.context > FLASH_NEXT_NATIVE_CONTEXT,
    };
    if enable_yarn {
        push(&["--enable-yarn"]);
    }
    push(&["--max-output-bytes", &cfg.max_output_bytes.to_string()]);
    push(&["--think", &cfg.think]);
    push(&["--temperature", &cfg.temperature.to_string()]);
    if cfg.max_tokens > 0 {
        push(&["--max-tokens", &cfg.max_tokens.to_string()]);
    }

    match speculative {
        Some(SpeculativeArg::Mtp(mtp)) => {
            push(&["--speculative", "mtp", "--mtp-model", &mtp.to_string_lossy()]);
        }
        Some(SpeculativeArg::Dspark(dspark)) => {
            push(&[
                "--speculative",
                "dspark",
                "--dspark-model",
                &dspark.to_string_lossy(),
            ]);
        }
        None => {}
    }

    if let Some(dir) = cache_dir {
        push(&["--cache-disk", &dir.to_string_lossy()]);
        if cfg.cache_disk_bytes > 0 {
            push(&["--cache-disk-bytes", &cfg.cache_disk_bytes.to_string()]);
        }
    }
    if let Some(vision) = vision {
        push(&["--mmproj", &vision.to_string_lossy()]);
    }
    if !cfg.extra_args.trim().is_empty() {
        args.extend(split_cli_args(&cfg.extra_args));
    }
    args
}

/// 產生可安全寫入日誌的指令字串 (遮蔽 API Key)。
pub fn redacted_command(program: &Path, args: &[String]) -> String {
    let mut out = vec![quote_arg(&program.to_string_lossy())];
    let mut mask_next = false;
    for arg in args {
        if mask_next {
            out.push("******".to_string());
            mask_next = false;
        } else {
            mask_next = arg == "--api-key";
            out.push(quote_arg(arg));
        }
    }
    out.join(" ")
}

fn quote_arg(arg: &str) -> String {
    if arg.contains(' ') {
        format!("\"{}\"", arg)
    } else {
        arg.to_string()
    }
}

#[derive(Default)]
struct Runtime {
    state: ServerStateEnum,
    child: Option<Child>,
    adopted: bool,
    start_time: Option<Instant>,
    error_message: Option<String>,
    https_proxy: Option<HttpsProxyHandle>,
    https_cert_path: Option<String>,
}

impl Runtime {
    /// 子程序結束時更新狀態，回傳 (新狀態, 日誌訊息)；若是預期中的停止則回傳 None。
    fn on_child_exited(&mut self, status: ExitStatus) -> Option<(ServerStateEnum, String)> {
        self.child = None;
        self.start_time = None;
        self.adopted = false;
        self.https_proxy = None;
        self.https_cert_path = None;
        match self.state {
            ServerStateEnum::Starting => {
                let msg = format!(
                    "Gufo 推理伺服器在啟動階段意外中止 ({})。請查看上方日誌，常見原因：模型路徑錯誤、記憶體不足或 Context × Sessions 過大。",
                    status
                );
                self.state = ServerStateEnum::Error;
                self.error_message = Some(msg.clone());
                Some((ServerStateEnum::Error, format!("[錯誤] {}", msg)))
            }
            ServerStateEnum::Running => {
                self.state = ServerStateEnum::Stopped;
                Some((
                    ServerStateEnum::Stopped,
                    format!(
                        "[系統] ⚠️ 偵測到 Gufo 推理伺服器已意外中止 ({})，狀態已自動重置為已停止。",
                        status
                    ),
                ))
            }
            _ => None,
        }
    }
}

struct Shared {
    runtime: Mutex<Runtime>,
    config: Mutex<GufoServerConfig>,
    logs: Mutex<VecDeque<String>>,
    metrics: Mutex<VecDeque<InferenceMetrics>>,
    /// 每次啟動/接管/停止都會遞增；舊的監控與讀取任務發現不相符時即自行結束。
    generation: AtomicU64,
    /// 序列化 start/stop，避免托盤與 UI 同時操作造成重複啟動。
    op_lock: Mutex<()>,
}

impl Shared {
    fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// 必須在持有 runtime 鎖時呼叫，確保世代與狀態變更一致。
    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    async fn log(&self, app: Option<&AppHandle>, line: impl Into<String>) {
        let line = line.into();
        {
            let mut logs = self.logs.lock().await;
            if logs.len() >= MAX_LOG_LINES {
                logs.pop_front();
            }
            logs.push_back(line.clone());
        }
        if let Some(app) = app {
            let _ = app.emit("gufo-log", line);
        }
    }

    async fn push_metrics(&self, app: Option<&AppHandle>, metrics: InferenceMetrics) {
        {
            let mut m = self.metrics.lock().await;
            if m.len() >= MAX_METRICS {
                m.pop_front();
            }
            m.push_back(metrics.clone());
        }
        if let Some(app) = app {
            let _ = app.emit("gufo-metrics", metrics);
        }
    }

    async fn notify_state(&self, app: Option<&AppHandle>, state: &ServerStateEnum) {
        if let Some(app) = app {
            let (port, https_port) = {
                let cfg = self.config.lock().await;
                (cfg.port, cfg.https_enabled.then_some(cfg.https_port))
            };
            crate::tray::update_tray_status(app, state, port, https_port);
            let _ = app.emit("gufo-status-changed", state.as_str());
        }
    }

    /// 若仍屬於同一世代且處於 Starting，轉為 Running 並回傳載入耗時。
    async fn mark_running_if_starting(&self, generation: u64, app: Option<&AppHandle>) -> Option<Duration> {
        let elapsed = {
            let mut rt = self.runtime.lock().await;
            if self.current_generation() != generation || rt.state != ServerStateEnum::Starting {
                return None;
            }
            rt.state = ServerStateEnum::Running;
            rt.start_time.get_or_insert_with(Instant::now).elapsed()
        };
        self.notify_state(app, &ServerStateEnum::Running).await;
        self.log(
            app,
            format!("[系統] ✅ Gufo 推理伺服器已就緒 (模型載入耗時 {:.1} 秒)。", elapsed.as_secs_f64()),
        )
        .await;
        Some(elapsed)
    }
}

fn http_client(timeout_ms: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .unwrap_or_default()
}

async fn probe_status(client: &reqwest::Client, url: &str, api_key: &str) -> Option<reqwest::StatusCode> {
    let mut req = client.get(url);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }
    req.send().await.ok().map(|r| r.status())
}

/// 以 `/ready` 判斷模型是否可接受推論 (載入中回 503)；舊版 Gufo 無此端點時退回 `/health`。
async fn probe_ready(client: &reqwest::Client, host: &str, port: u16, api_key: &str) -> bool {
    let target = connect_host(host);
    let base = format!("http://{}:{}", format_url_host(&target), port);
    match probe_status(client, &format!("{}/ready", base), api_key).await {
        Some(s) if s.is_success() => true,
        Some(reqwest::StatusCode::NOT_FOUND) => {
            matches!(probe_status(client, &format!("{}/health", base), api_key).await, Some(s) if s.is_success())
        }
        _ => false,
    }
}

async fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    let target = connect_host(host);
    tokio::time::timeout(timeout, tokio::net::TcpStream::connect((target.as_str(), port)))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// 逐行讀取子程序輸出。使用 lossy 解碼，避免非 UTF-8 輸出 (例如系統錯誤訊息)
/// 讓讀取中斷進而塞滿管線、卡住 Gufo。
fn spawn_output_reader<R>(shared: Arc<Shared>, app: AppHandle, generation: u64, stream: R)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::with_capacity(512);
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&buf)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if shared.current_generation() == generation {
                if let Some(active) = ActiveInferenceRequest::parse_received_line(&line) {
                    let _ = app.emit("gufo-request-active", active);
                }
                if let Some(metrics) = InferenceMetrics::parse_log_line(&line) {
                    shared.push_metrics(Some(&app), metrics).await;
                }
                if let Some(req_id) = parse_completed_request_id(&line) {
                    let _ = app.emit("gufo-request-finished", req_id);
                }
            }
            let ready = is_listening_line(&line);
            shared.log(Some(&app), line).await;
            if ready {
                shared.mark_running_if_starting(generation, Some(&app)).await;
            }
        }
    });
}

/// 生命週期監控：啟動階段等待 `/ready`，運行階段偵測子程序崩潰或外部伺服器中止。
fn spawn_lifecycle_monitor(shared: Arc<Shared>, app: AppHandle, generation: u64, host: String, port: u16, api_key: String) {
    tokio::spawn(async move {
        let client = http_client(1500);
        let started = Instant::now();
        let mut hinted = false;

        // 第一階段：等待模型載入完成 (不設硬性逾時；Windows carve-out 不足時載入可能需數分鐘)
        loop {
            if shared.current_generation() != generation {
                return;
            }
            {
                let mut rt = shared.runtime.lock().await;
                if rt.state != ServerStateEnum::Starting {
                    break;
                }
                if let Some(child) = rt.child.as_mut() {
                    if let Ok(Some(status)) = child.try_wait() {
                        let transition = rt.on_child_exited(status);
                        drop(rt);
                        if let Some((state, msg)) = transition {
                            shared.log(Some(&app), msg).await;
                            shared.notify_state(Some(&app), &state).await;
                        }
                        return;
                    }
                } else if !rt.adopted {
                    return;
                }
            }

            if probe_ready(&client, &host, port, &api_key).await {
                shared.mark_running_if_starting(generation, Some(&app)).await;
                break;
            }
            if !hinted && started.elapsed() > SLOW_LOAD_HINT_AFTER {
                hinted = true;
                shared
                    .log(
                        Some(&app),
                        "[系統] 模型仍在載入中…若 BIOS 的 iGPU 記憶體配置 (UMA carve-out) 低於 96GB，Windows 上載入可能需要數分鐘，且解碼速度會下降。",
                    )
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // 第二階段：持續監控
        let mut adopted_failures = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if shared.current_generation() != generation {
                return;
            }
            let mut rt = shared.runtime.lock().await;
            if rt.state != ServerStateEnum::Running {
                return;
            }
            if let Some(child) = rt.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    let transition = rt.on_child_exited(status);
                    drop(rt);
                    if let Some((state, msg)) = transition {
                        shared.log(Some(&app), msg).await;
                        shared.notify_state(Some(&app), &state).await;
                    }
                    return;
                }
                continue;
            }
            if !rt.adopted {
                return;
            }
            drop(rt);

            if port_open(&host, port, Duration::from_millis(500)).await {
                adopted_failures = 0;
                continue;
            }
            adopted_failures += 1;
            if adopted_failures < 2 {
                continue;
            }
            let mut rt = shared.runtime.lock().await;
            if shared.current_generation() != generation || rt.state != ServerStateEnum::Running || !rt.adopted {
                return;
            }
            rt.state = ServerStateEnum::Stopped;
            rt.adopted = false;
            rt.start_time = None;
            rt.https_proxy = None;
            rt.https_cert_path = None;
            drop(rt);
            shared
                .log(Some(&app), "[系統] ⚠️ 偵測到外部 Gufo 推理伺服器已中止，狀態已自動重置為已停止。")
                .await;
            shared.notify_state(Some(&app), &ServerStateEnum::Stopped).await;
            return;
        }
    });
}

/// 先送出 Ctrl+C 讓 Gufo 正常關閉，逾時才強制終止。
async fn shutdown_child(mut child: Child) -> String {
    if let Ok(Some(status)) = child.try_wait() {
        return format!("[系統] Gufo 子程序已結束 ({})。", status);
    }
    if let Some(pid) = child.id() {
        let sent = tokio::task::spawn_blocking(move || process_control::request_graceful_stop(pid))
            .await
            .unwrap_or(false);
        if sent {
            if let Ok(Ok(status)) = tokio::time::timeout(GRACEFUL_STOP_TIMEOUT, child.wait()).await {
                return format!("[系統] Gufo 已正常關閉 ({})，GPU 記憶體已釋放。", status);
            }
        }
    }
    let _ = child.kill().await;
    "[系統] Gufo 未在時限內正常結束，已強制終止子程序，GPU 記憶體已釋放。".to_string()
}

pub struct GufoManager {
    shared: Arc<Shared>,
}

impl Default for GufoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GufoManager {
    pub fn new() -> Self {
        let initial_config = Self::load_saved_config().unwrap_or_default();
        Self {
            shared: Arc::new(Shared {
                runtime: Mutex::new(Runtime::default()),
                config: Mutex::new(initial_config),
                logs: Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES)),
                metrics: Mutex::new(VecDeque::with_capacity(MAX_METRICS)),
                generation: AtomicU64::new(0),
                op_lock: Mutex::new(()),
            }),
        }
    }

    pub fn app_base_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn get_config_file_path() -> PathBuf {
        Self::app_base_dir().join("gufo-studio.config.json")
    }

    pub fn has_saved_config() -> bool {
        Self::get_config_file_path().exists()
    }

    pub fn load_saved_config() -> Option<GufoServerConfig> {
        let content = std::fs::read_to_string(Self::get_config_file_path()).ok()?;
        serde_json::from_str::<GufoServerConfig>(&content).ok()
    }

    pub async fn save_config(&self, app: Option<&AppHandle>, cfg: GufoServerConfig) -> Result<(), String> {
        *self.shared.config.lock().await = cfg.clone();
        if let Some(app_handle) = app {
            let current_state = self.shared.runtime.lock().await.state.clone();
            let https_port = cfg.https_enabled.then_some(cfg.https_port);
            crate::tray::update_tray_status(app_handle, &current_state, cfg.port, https_port);
        }
        let path = Self::get_config_file_path();
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("無法寫入設定檔 {}: {}", path.display(), e))?;
        Ok(())
    }

    pub async fn get_config(&self) -> GufoServerConfig {
        self.shared.config.lock().await.clone()
    }

    pub async fn get_status(&self, app: Option<&AppHandle>) -> ServerStatus {
        let cfg = self.shared.config.lock().await.clone();
        let bind_host = effective_bind_host(&cfg.host);
        let client_host = format_url_host(&connect_host(&bind_host));
        let lan_ip = if is_wildcard_host(&bind_host) {
            https_proxy::parse_san_list(&cfg.https_sans)
                .into_iter()
                .find(|entry| entry.parse::<std::net::IpAddr>().is_ok())
                .or_else(detect_local_ipv4)
        } else {
            None
        };

        let (status, transition) = {
            let mut rt = self.shared.runtime.lock().await;
            // 主動檢查子程序是否已意外終止 (監控任務也會偵測，此處讓輪詢立即反映)
            let exited = rt.child.as_mut().and_then(|c| c.try_wait().ok().flatten());
            let transition = exited.and_then(|status| rt.on_child_exited(status));
            let uptime = match (&rt.state, rt.start_time) {
                (ServerStateEnum::Running, Some(t)) => t.elapsed().as_secs(),
                _ => 0,
            };
            let https_running = rt.https_proxy.is_some();
            let https_cert_path = rt.https_cert_path.clone().or_else(|| {
                if !cfg.https_enabled {
                    return None;
                }
                if !cfg.https_cert_path.trim().is_empty() {
                    Some(Self::resolve_path(cfg.https_cert_path.trim()).to_string_lossy().to_string())
                } else {
                    let (default_cert, _) = https_proxy::default_cert_paths(&Self::app_base_dir());
                    default_cert.exists().then(|| default_cert.to_string_lossy().to_string())
                }
            });
            let status = ServerStatus {
                state: rt.state.clone(),
                pid: rt.child.as_ref().and_then(|c| c.id()),
                port: cfg.port,
                host: bind_host.clone(),
                served_model_name: cfg.served_model_name.clone(),
                endpoint: format!("http://{}:{}/v1", client_host, cfg.port),
                uptime_secs: uptime,
                error_message: rt.error_message.clone(),
                api_key: Some(cfg.api_key.trim().to_string()).filter(|k| !k.is_empty()),
                https_enabled: cfg.https_enabled,
                https_port: cfg.https_port,
                https_running,
                https_endpoint: cfg
                    .https_enabled
                    .then(|| format!("https://{}:{}/v1", client_host, cfg.https_port)),
                https_cert_path,
                lan_ip,
            };
            (status, transition)
        };

        if let Some((state, msg)) = transition {
            self.shared.log(app, msg).await;
            self.shared.notify_state(app, &state).await;
        }
        status
    }

    pub async fn get_logs(&self) -> Vec<String> {
        self.shared.logs.lock().await.iter().cloned().collect()
    }

    pub async fn get_metrics(&self) -> Vec<InferenceMetrics> {
        self.shared.metrics.lock().await.iter().cloned().collect()
    }

    pub fn resolve_path(path_str: &str) -> PathBuf {
        let p = Path::new(path_str);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        let cwd = std::env::current_dir().ok();
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        for base in [&cwd, &exe_dir].into_iter().flatten() {
            let candidate = base.join(p);
            if candidate.exists() {
                return candidate;
            }
        }
        cwd.map(|c| c.join(p)).unwrap_or_else(|| p.to_path_buf())
    }

    fn collect_required_sans(bind_host: &str, https_sans: &str) -> Vec<String> {
        let mut sans = https_proxy::parse_san_list(https_sans);
        if !is_wildcard_host(bind_host) && !sans.iter().any(|s| s == bind_host) {
            sans.push(bind_host.to_string());
        }
        sans
    }

    async fn start_https_for_config(
        &self,
        cfg: &GufoServerConfig,
    ) -> Result<Option<(HttpsProxyHandle, String, Vec<String>)>, String> {
        if !cfg.https_enabled {
            return Ok(None);
        }
        if cfg.https_port == 0 {
            return Err("HTTPS Port 不可為 0。".to_string());
        }
        if cfg.https_port == cfg.port {
            return Err(format!(
                "HTTP Port ({}) 與 HTTPS Port ({}) 不可相同，請更換其中一個埠號。",
                cfg.port, cfg.https_port
            ));
        }

        let bind_host = effective_bind_host(&cfg.host);
        let upstream_host = connect_host(&bind_host);
        let extra_sans = Self::collect_required_sans(&bind_host, &cfg.https_sans);

        let prepared = https_proxy::prepare_tls_server_config(
            &cfg.https_cert_path,
            &cfg.https_key_path,
            &Self::app_base_dir(),
            &extra_sans,
            Self::resolve_path,
        )?;

        let api_key_opt = Some(cfg.api_key.trim().to_string()).filter(|k| !k.is_empty());
        let handle = https_proxy::start_https_proxy(
            &bind_host,
            cfg.https_port,
            &upstream_host,
            cfg.port,
            prepared.server_config,
            api_key_opt.clone(),
        )
        .await?;

        let cert_path_str = prepared.cert_path.to_string_lossy().to_string();
        let mut logs = Vec::new();
        if prepared.newly_created {
            logs.push(format!(
                "[系統] 🔐 已自動產生本機 HTTPS 自簽憑證：{}",
                cert_path_str
            ));
        }
        let auth_suffix = if api_key_opt.is_some() {
            "（已啟用 Bearer API Key 驗證）"
        } else {
            ""
        };
        logs.push(format!(
            "[系統] 🔒 HTTPS 反向代理已監聽 https://{}:{}/v1 -> 轉發至 http://{}:{}/v1{}",
            format_url_host(&bind_host),
            cfg.https_port,
            format_url_host(&upstream_host),
            cfg.port,
            auth_suffix
        ));

        Ok(Some((handle, cert_path_str, logs)))
    }

    pub async fn regenerate_https_cert(
        &self,
        override_host: Option<String>,
        override_sans: Option<String>,
    ) -> Result<String, String> {
        let cfg = self.get_config().await;
        let bind_host = effective_bind_host(override_host.as_deref().unwrap_or(&cfg.host));
        let sans_raw = override_sans.as_deref().unwrap_or(&cfg.https_sans);
        let extra_sans = Self::collect_required_sans(&bind_host, sans_raw);
        let (cert_path, key_path) = https_proxy::default_cert_paths(&Self::app_base_dir());
        https_proxy::generate_self_signed_cert_files(&cert_path, &key_path, &extra_sans)?;
        Ok(cert_path.to_string_lossy().to_string())
    }

    /// 將指定 Port 上已就緒的 Gufo 伺服器標記為「已接管」並開始監控。
    async fn adopt_running_server(&self, app: AppHandle, cfg: &GufoServerConfig, message: String) {
        let https_outcome = self.start_https_for_config(cfg).await;
        let mut extra_logs = Vec::new();
        let (https_handle, https_cert_path) = match https_outcome {
            Ok(Some((handle, cert_path, logs))) => {
                extra_logs = logs;
                (Some(handle), Some(cert_path))
            }
            Ok(None) => (None, None),
            Err(err) => {
                extra_logs.push(format!(
                    "[警告] HTTPS 反向代理啟動失敗（已接管的 HTTP 服務仍維持運行）：{}",
                    err
                ));
                (None, None)
            }
        };

        let generation = {
            let mut rt = self.shared.runtime.lock().await;
            let generation = self.shared.next_generation();
            rt.state = ServerStateEnum::Running;
            rt.adopted = true;
            rt.start_time = Some(Instant::now());
            rt.error_message = None;
            rt.https_proxy = https_handle;
            rt.https_cert_path = https_cert_path;
            generation
        };
        self.shared.log(Some(&app), message).await;
        for line in extra_logs {
            self.shared.log(Some(&app), line).await;
        }
        self.shared.notify_state(Some(&app), &ServerStateEnum::Running).await;
        spawn_lifecycle_monitor(
            self.shared.clone(),
            app,
            generation,
            connect_host(&cfg.host),
            cfg.port,
            cfg.api_key.trim().to_string(),
        );
    }

    pub async fn sync_existing_server_on_startup(&self, app: AppHandle) {
        let _op = self.shared.op_lock.lock().await;
        let cfg = self.get_config().await;
        if self.shared.runtime.lock().await.state != ServerStateEnum::Stopped {
            return;
        }
        if probe_ready(&http_client(800), &cfg.host, cfg.port, cfg.api_key.trim()).await {
            let msg = format!("[系統] 偵測到 Port {} 已有運行的 Gufo 伺服器，已自動同步運行狀態。", cfg.port);
            self.adopt_running_server(app, &cfg, msg).await;
        }
    }

    async fn fail(&self, app: &AppHandle, msg: String) -> Result<ServerStatus, String> {
        {
            let mut rt = self.shared.runtime.lock().await;
            self.shared.next_generation();
            rt.state = ServerStateEnum::Error;
            rt.error_message = Some(msg.clone());
            rt.start_time = None;
            rt.https_proxy = None;
            rt.https_cert_path = None;
        }
        self.shared.log(Some(app), format!("[錯誤] {}", msg)).await;
        self.shared.notify_state(Some(app), &ServerStateEnum::Error).await;
        Err(msg)
    }

    pub async fn start(&self, app: AppHandle, new_config: Option<GufoServerConfig>) -> Result<ServerStatus, String> {
        let _op = self.shared.op_lock.lock().await;
        {
            let mut rt = self.shared.runtime.lock().await;
            if let Some(child) = rt.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(_)) => rt.child = None,
                    _ => return Err("伺服器已在運行中".to_string()),
                }
            }
        }

        if let Some(cfg) = new_config {
            *self.shared.config.lock().await = cfg;
        }
        let cfg = self.get_config().await;
        let port = cfg.port;
        let api_key = cfg.api_key.trim().to_string();

        // 目標 Port 已有就緒的 Gufo 伺服器 (例如先前實例啟動) → 直接接管，避免重複佔用記憶體
        if probe_ready(&http_client(800), &cfg.host, port, &api_key).await {
            let msg = format!(
                "[系統] 偵測到 Port {} 已有運行的 Gufo 伺服器，已自動接管該服務，避免重複啟動耗損顯存。",
                port
            );
            self.adopt_running_server(app.clone(), &cfg, msg).await;
            return Ok(self.get_status(Some(&app)).await);
        }

        if port_open(&cfg.host, port, Duration::from_millis(500)).await {
            let msg = format!(
                "Port {} 已被佔用但尚未通過就緒檢查（可能是仍在載入模型的 Gufo，或其他程式）。請稍候再試或更換 Port。",
                port
            );
            return self.fail(&app, msg).await;
        }

        let resolved_gufo = Self::resolve_path(&cfg.gufo_path);
        if !resolved_gufo.exists() {
            return self.fail(&app, format!("找不到 Gufo 主程式檔案：{}", cfg.gufo_path)).await;
        }
        let resolved_model = Self::resolve_path(&cfg.model_path);
        if !resolved_model.exists() {
            return self.fail(&app, format!("找不到模型主檔：{}", cfg.model_path)).await;
        }

        let mut warnings = Vec::new();
        let speculative_mode = cfg.speculative.trim().to_ascii_lowercase();
        let mtp_path_buf = if speculative_mode == "mtp" && !cfg.mtp_path.trim().is_empty() {
            let p = Self::resolve_path(&cfg.mtp_path);
            if p.exists() {
                Some(p)
            } else {
                warnings.push(format!(
                    "[警告] 找不到 MTP 模型 {}，本次將不使用投機解碼（生成速度會明顯下降）。",
                    cfg.mtp_path
                ));
                None
            }
        } else {
            None
        };
        let dspark_path_buf = if speculative_mode == "dspark" && !cfg.dspark_path.trim().is_empty()
        {
            let p = Self::resolve_path(&cfg.dspark_path);
            if p.exists() {
                Some(p)
            } else {
                warnings.push(format!(
                    "[警告] 找不到 DSpark 模型 {}，本次將不使用投機解碼。",
                    cfg.dspark_path
                ));
                None
            }
        } else {
            None
        };
        let speculative_arg = match (mtp_path_buf.as_deref(), dspark_path_buf.as_deref()) {
            (Some(p), _) => Some(SpeculativeArg::Mtp(p)),
            (_, Some(p)) => Some(SpeculativeArg::Dspark(p)),
            _ => None,
        };
        let vision = if !cfg.vision_path.trim().is_empty() {
            let p = Self::resolve_path(&cfg.vision_path);
            if p.exists() {
                Some(p)
            } else {
                warnings.push(format!("[警告] 找不到視覺模型 {}，本次將不啟用圖片輸入。", cfg.vision_path));
                None
            }
        } else {
            None
        };
        let cache_dir = if cfg.cache_disk {
            let dir_str = if cfg.cache_disk_dir.trim().is_empty() { "gufo-cache" } else { cfg.cache_disk_dir.trim() };
            let dir = Self::resolve_path(dir_str);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return self.fail(&app, format!("無法建立磁碟快取目錄 {}：{}", dir.display(), e)).await;
            }
            Some(dir)
        } else {
            None
        };

        let (https_handle, https_cert_path, https_logs) = match self.start_https_for_config(&cfg).await {
            Ok(Some((handle, cert_path, logs))) => (Some(handle), Some(cert_path), logs),
            Ok(None) => (None, None, Vec::new()),
            Err(e) => return self.fail(&app, e).await,
        };

        let generation = {
            let mut rt = self.shared.runtime.lock().await;
            let generation = self.shared.next_generation();
            rt.state = ServerStateEnum::Starting;
            rt.adopted = false;
            rt.error_message = None;
            rt.start_time = None;
            rt.https_proxy = https_handle;
            rt.https_cert_path = https_cert_path;
            generation
        };
        self.shared.notify_state(Some(&app), &ServerStateEnum::Starting).await;
        for w in warnings {
            self.shared.log(Some(&app), w).await;
        }
        for line in https_logs {
            self.shared.log(Some(&app), line).await;
        }

        let args = build_args(
            &cfg,
            &resolved_model,
            speculative_arg,
            vision.as_deref(),
            cache_dir.as_deref(),
        );
        self.shared
            .log(Some(&app), format!("[系統] 準備啟動：{}", redacted_command(&resolved_gufo, &args)))
            .await;

        let mut command = Command::new(&resolved_gufo);
        if let Ok(cwd) = std::env::current_dir() {
            command.current_dir(cwd);
        }
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // CREATE_NO_WINDOW：隱藏主控台視窗，但保留主控台以便停止時送出 Ctrl+C。
        // 不可加上 CREATE_NEW_PROCESS_GROUP，否則 Ctrl+C 會被停用。
        #[cfg(target_os = "windows")]
        command.creation_flags(0x0800_0000);

        let mut spawned = match command.spawn() {
            Ok(c) => c,
            Err(e) => return self.fail(&app, format!("無法啟動 Gufo 程序: {}", e)).await,
        };
        let stdout = spawned.stdout.take();
        let stderr = spawned.stderr.take();
        {
            let mut rt = self.shared.runtime.lock().await;
            rt.child = Some(spawned);
            rt.start_time = Some(Instant::now());
        }

        if let Some(stdout) = stdout {
            spawn_output_reader(self.shared.clone(), app.clone(), generation, stdout);
        }
        if let Some(stderr) = stderr {
            spawn_output_reader(self.shared.clone(), app.clone(), generation, stderr);
        }
        spawn_lifecycle_monitor(self.shared.clone(), app.clone(), generation, connect_host(&cfg.host), port, api_key);

        Ok(self.get_status(Some(&app)).await)
    }

    /// 停止伺服器。只會終止本程式啟動的子程序，或在設定 Port 上監聽、且確認為 Gufo 的已接管程序；
    /// 不會以映像名稱終止其他 Gufo 實例。
    pub async fn stop(&self, app: Option<&AppHandle>) -> Result<ServerStatus, String> {
        let _op = self.shared.op_lock.lock().await;
        let (child, adopted, https_proxy) = {
            let mut rt = self.shared.runtime.lock().await;
            self.shared.next_generation();
            let child = rt.child.take();
            let adopted = rt.adopted;
            let https_proxy = rt.https_proxy.take();
            rt.state = ServerStateEnum::Stopped;
            rt.adopted = false;
            rt.start_time = None;
            rt.error_message = None;
            rt.https_cert_path = None;
            (child, adopted, https_proxy)
        };
        if let Some(mut proxy) = https_proxy {
            proxy.shutdown();
            self.shared
                .log(app, "[系統] 🔒 HTTPS 反向代理已停止。")
                .await;
        }
        let cfg = self.get_config().await;
        let mut result = Ok(());

        if let Some(child) = child {
            self.shared
                .log(app, "[系統] 正在停止 Gufo 推理引擎（先嘗試正常關閉以保存快取）...")
                .await;
            let msg = shutdown_child(child).await;
            self.shared.log(app, msg).await;
        } else if adopted {
            self.shared
                .log(app, format!("[系統] 正在停止已接管的 Gufo 伺服器 (Port {})...", cfg.port))
                .await;
            let expected = process_control::expected_gufo_images(&cfg.gufo_path);
            let port = cfg.port;
            let outcome = tokio::task::spawn_blocking(move || {
                process_control::stop_gufo_on_port(port, &expected, GRACEFUL_STOP_TIMEOUT)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            match outcome {
                Ok(msg) => self.shared.log(app, format!("[系統] {}", msg)).await,
                Err(msg) => {
                    self.shared.log(app, format!("[錯誤] {}", msg)).await;
                    result = Err(msg);
                }
            }
        }

        self.shared.notify_state(app, &ServerStateEnum::Stopped).await;
        result?;
        Ok(self.get_status(app).await)
    }

    pub async fn check_health(&self) -> bool {
        let cfg = self.get_config().await;
        probe_ready(&http_client(800), &cfg.host, cfg.port, cfg.api_key.trim()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_default_config() {
        let config = GufoServerConfig::default();
        assert_eq!(config.port, 12223);
        assert_eq!(config.host, "0.0.0.0");
        assert!(config.https_enabled);
        assert_eq!(config.https_port, 12443);
        assert_eq!(config.https_cert_path, "");
        assert_eq!(config.https_key_path, "");
        assert_eq!(config.https_sans, "");
        assert_eq!(config.context, 131072);
        assert_eq!(config.sessions, 2);
        assert_eq!(config.served_model_name, "flash-next");
        assert_eq!(config.think, "off");
        assert_eq!(config.temperature, 0.0);
        assert_eq!(config.speculative, "mtp");
        assert_eq!(config.mtp_policy, "survival");
        assert!(config.prompt_lookup);
        assert_eq!(config.api_key, "");
        assert!(config.auto_open_gui);
        assert!(!config.auto_start_server);
        assert_eq!(config.max_tokens, 32768);
        assert_eq!(config.mtp_draft_vocab, "full");
        assert!(config.cache_disk);

        // 預設路徑應為相對工作目錄，不應寫死 D:\Agents 或 D:\Models
        assert!(!config.gufo_path.starts_with(r"D:\Agents\gufo"));
        assert!(!config.model_path.starts_with(r"D:\Models\GGUF"));
    }

    #[test]
    fn test_legacy_config_json_gets_new_defaults() {
        let legacy = r#"{"gufo_path":"gufo.exe","model_path":"m.gguf","mtp_path":"","vision_path":"",
            "served_model_name":"x","host":"127.0.0.1","port":9000,"context":32768,"sessions":1,
            "max_request_bytes":1,"max_output_bytes":1,"think":"on","temperature":0.7,
            "speculative":"mtp","mtp_policy":"length","prompt_lookup":false}"#;
        let cfg: GufoServerConfig = serde_json::from_str(legacy).unwrap();
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.think, "on");
        assert!(cfg.auto_open_gui);
        assert_eq!(cfg.max_tokens, 32768);
        assert!(cfg.cache_disk);
        assert_eq!(cfg.cache_disk_dir, "gufo-cache");
        assert!(cfg.https_enabled);
        assert_eq!(cfg.https_port, 12443);
        assert_eq!(cfg.yarn_mode, "auto");
        assert!(cfg.dspark_path.is_empty());
        assert!(cfg.extra_args.is_empty());
        assert!(cfg.model_profiles.is_empty());
    }

    #[test]
    fn test_build_args_official_gufo_cli_and_yarn() {
        let mut cfg = GufoServerConfig::default();
        cfg.api_key = "sk-gufo-secret".to_string();
        cfg.draft_tokens = 5;
        cfg.mtp_draft_vocab = "latin".to_string();
        cfg.prefill_chunk = 1024;
        cfg.cache_disk_bytes = 1 << 30;
        let args = build_args(
            &cfg,
            Path::new("model.gguf"),
            Some(SpeculativeArg::Mtp(Path::new("mtp.gguf"))),
            None,
            Some(Path::new("cache dir")),
        );
        let joined = args.join(" ");
        for expected in [
            "--host 0.0.0.0",
            "--max-tokens 32768",
            "--speculative mtp --mtp-model mtp.gguf",
            "--cache-disk cache dir",
            "--cache-disk-bytes 1073741824",
        ] {
            assert!(joined.contains(expected), "missing `{}` in `{}`", expected, joined);
        }
        for removed in [
            "--api-key",
            "--mtp-policy",
            "--mtp-draft-vocab",
            "--prompt-lookup",
            "--draft-tokens",
            "--prefill-chunk",
            "--enable-yarn",
            "--mmproj",
        ] {
            assert!(!joined.contains(removed), "unexpected `{}` in `{}`", removed, joined);
        }

        cfg.context = 409_600;
        let yarn_args = build_args(&cfg, Path::new("model.gguf"), None, None, None).join(" ");
        assert!(yarn_args.contains("--context 409600"));
        assert!(yarn_args.contains("--enable-yarn"));

        let shown = redacted_command(Path::new("gufo.exe"), &args);
        assert!(shown.contains("\"cache dir\""));
        let masked = redacted_command(
            Path::new("gufo.exe"),
            &["--api-key".to_string(), "sk-gufo-secret".to_string()],
        );
        assert!(!masked.contains("sk-gufo-secret"));
        assert!(masked.contains("--api-key ******"));
    }

    #[test]
    fn test_build_args_inkling2_dspark_yarn_off_and_extra_args() {
        let cfg = GufoServerConfig {
            served_model_name: "inkling-2-mini".to_string(),
            context: 1_048_576,
            sessions: 4,
            speculative: "dspark".to_string(),
            yarn_mode: "off".to_string(),
            extra_args: r#"--extension "D:\Models\Inkling 2\ext.dll" --log-level debug"#.to_string(),
            ..Default::default()
        };
        let args = build_args(
            &cfg,
            Path::new("Inkling-2-Mini-UD-Q4_K_XL.gguf"),
            Some(SpeculativeArg::Dspark(Path::new("Inkling-2-Mini-dspark-Q8_0.gguf"))),
            Some(Path::new("mmproj-BF16.gguf")),
            None,
        );
        let joined = args.join(" ");
        assert!(joined.contains("--served-model-name inkling-2-mini"));
        assert!(joined.contains("--context 1048576"));
        assert!(!joined.contains("--enable-yarn"));
        assert!(joined.contains("--speculative dspark --dspark-model Inkling-2-Mini-dspark-Q8_0.gguf"));
        assert!(joined.contains("--mmproj mmproj-BF16.gguf"));
        assert!(args.contains(&"--extension".to_string()));
        assert!(args.contains(&r"D:\Models\Inkling 2\ext.dll".to_string()));
        assert!(args.contains(&"--log-level".to_string()));
        assert!(args.contains(&"debug".to_string()));
    }

    #[test]
    fn test_yarn_mode_auto_on_off() {
        let auto_low = GufoServerConfig {
            context: 131_072,
            yarn_mode: "auto".to_string(),
            ..Default::default()
        };
        assert!(!build_args(&auto_low, Path::new("m.gguf"), None, None, None)
            .contains(&"--enable-yarn".to_string()));

        let force_on = GufoServerConfig {
            context: 65_536,
            yarn_mode: "on".to_string(),
            ..Default::default()
        };
        assert!(build_args(&force_on, Path::new("m.gguf"), None, None, None)
            .contains(&"--enable-yarn".to_string()));

        let force_off = GufoServerConfig {
            context: 1_048_576,
            yarn_mode: "off".to_string(),
            ..Default::default()
        };
        assert!(!build_args(&force_off, Path::new("m.gguf"), None, None, None)
            .contains(&"--enable-yarn".to_string()));
    }

    #[test]
    fn test_split_cli_args_windows_paths_and_quotes() {
        let parsed = split_cli_args(
            r#"--flag-a "C:\Program Files\Gufo\ext.dll" --flag-b 'single quoted' --plain=123"#,
        );
        assert_eq!(
            parsed,
            vec![
                "--flag-a",
                r"C:\Program Files\Gufo\ext.dll",
                "--flag-b",
                "single quoted",
                "--plain=123",
            ]
        );
    }

    #[test]
    fn test_build_args_omits_unset_options() {
        let mut cfg = GufoServerConfig::default();
        cfg.max_tokens = 0;
        let args = build_args(&cfg, Path::new("model.gguf"), None, None, None).join(" ");
        for absent in [
            "--api-key",
            "--max-tokens",
            "--speculative",
            "--draft-tokens",
            "--mtp-draft-vocab",
            "--prefill-chunk",
            "--cache-disk",
            "--enable-yarn",
        ] {
            assert!(!args.contains(absent), "unexpected `{}` in `{}`", absent, args);
        }
    }

    #[test]
    fn test_listening_detection_ignores_errors() {
        assert!(is_listening_line("2026-01-01 00:00:00 [INFO] [http] event=listening address=127.0.0.1:12223"));
        assert!(!is_listening_line("[ERROR] [http] Error starting HTTP server: address in use"));
        assert!(!is_listening_line("serving model flash-next"));
    }

    #[tokio::test]
    async fn test_manager_initial_status() {
        let manager = GufoManager::new();
        let status = manager.get_status(None).await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.host, "0.0.0.0");
        assert_eq!(status.port, 12223);
        assert_eq!(status.pid, None);
        assert_eq!(status.endpoint, "http://127.0.0.1:12223/v1");
        assert!(status.https_enabled);
        assert_eq!(status.https_port, 12443);
        assert!(!status.https_running);
        assert_eq!(status.https_endpoint.as_deref(), Some("https://127.0.0.1:12443/v1"));

        // 未啟動也未接管時，stop 不應終止任何程序，也不會死鎖
        let stop_status = manager.stop(None).await.unwrap();
        assert_eq!(stop_status.state, ServerStateEnum::Stopped);
    }

    async fn spawn_exiting_child() -> Child {
        #[cfg(target_os = "windows")]
        let child = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        #[cfg(not(target_os = "windows"))]
        let child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        child
    }

    #[tokio::test]
    async fn test_unexpected_process_exit_resets_status() {
        let manager = GufoManager::new();
        {
            let mut rt = manager.shared.runtime.lock().await;
            rt.child = Some(spawn_exiting_child().await);
            rt.state = ServerStateEnum::Running;
            rt.start_time = Some(Instant::now());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        let status = manager.get_status(None).await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.pid, None);
        assert_eq!(status.uptime_secs, 0);
    }

    #[tokio::test]
    async fn test_exit_during_startup_reports_error() {
        let manager = GufoManager::new();
        {
            let mut rt = manager.shared.runtime.lock().await;
            rt.child = Some(spawn_exiting_child().await);
            rt.state = ServerStateEnum::Starting;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;

        let status = manager.get_status(None).await;
        assert_eq!(status.state, ServerStateEnum::Error);
        assert!(status.error_message.is_some());
    }

    #[tokio::test]
    async fn test_resolve_relative_path() {
        let p = GufoManager::resolve_path("Cargo.toml");
        assert!(p.ends_with("Cargo.toml"));
    }

    #[test]
    fn test_parse_metrics_log_line() {
        let sample = "2026-10-24 14:00:00 [INFO] [http] request=r2 event=completed method=POST path=/v1/chat/completions status=200 duration_ms=4520.5 outcome=completed prompt_tokens=512 prefill_tokens=512 generated_tokens=159 finish=stop cache=miss cached_tokens=0 queue_ms=0.1 ttft_ms=12.5 prefill_tps=1240.5 decode_tps=31.8 batch_width=1 draft_accepted=85 draft_proposed=100 acceptance_pct=85.0";
        let parsed = InferenceMetrics::parse_log_line(sample).expect("Should parse successfully");
        assert_eq!(parsed.request_id, "r2");
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.path, "/v1/chat/completions");
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.prompt_tokens, 512);
        assert_eq!(parsed.prefill_tokens, 512);
        assert_eq!(parsed.generated_tokens, 159);
        assert_eq!(parsed.outcome, "completed");
        assert_eq!(parsed.finish, "stop");
        assert_eq!(parsed.cache, "miss");
        assert_eq!(parsed.cached_tokens, 0);
        assert!((parsed.cache_hit_pct - 0.0).abs() < 0.01);
        assert!((parsed.prefill_tps - 1240.5).abs() < 0.01);
        assert!((parsed.decode_tps - 31.8).abs() < 0.01);
        assert!((parsed.ttft_ms - 12.5).abs() < 0.01);
        assert_eq!(parsed.draft_accepted, 85);
        assert_eq!(parsed.draft_proposed, 100);
        assert_eq!(parsed.acceptance_pct, Some(85.0));
    }

    #[test]
    fn test_parse_metrics_cache_hit_and_miss_diagnostics() {
        let memory_hit = "2026-11-19 22:00:00 [INFO] [http] request=r8 event=completed method=POST path=/v1/chat/completions status=200 duration_ms=1800.0 outcome=completed prompt_tokens=20000 prefill_tokens=800 generated_tokens=120 finish=stop cache=memory cached_tokens=19200 cache_restore_ms=14.2 queue_ms=0.1 ttft_ms=650.0 prefill_tps=1280.0 decode_tps=42.5 batch_width=1 draft_accepted=60 draft_proposed=80 acceptance_pct=75.0 lookup_accepted=12 lookup_proposed=16";
        let parsed = InferenceMetrics::parse_log_line(memory_hit).expect("memory hit metrics");
        assert_eq!(parsed.cache, "memory");
        assert_eq!(parsed.cached_tokens, 19200);
        assert_eq!(parsed.prefill_tokens, 800);
        assert!((parsed.cache_hit_pct - 96.0).abs() < 0.01);
        assert!((parsed.cache_restore_ms - 14.2).abs() < 0.01);
        assert_eq!(parsed.lookup_accepted, 12);
        assert_eq!(parsed.lookup_proposed, 16);
        assert_eq!(parsed.cache_miss_reason, None);

        let disk_hit = "2026-11-19 22:01:00 [INFO] [http] request=r9 event=completed method=POST path=/v1/responses status=200 duration_ms=2100.0 outcome=completed prompt_tokens=10000 prefill_tokens=0 generated_tokens=50 finish=length cache=disk cached_tokens=10000 cache_restore_ms=85.5 queue_ms=0.2 ttft_ms=90.0 prefill_tps=0.0 decode_tps=40.0 batch_width=1 draft_accepted=0 draft_proposed=0";
        let parsed_disk = InferenceMetrics::parse_log_line(disk_hit).expect("disk hit metrics");
        assert_eq!(parsed_disk.cache, "disk");
        assert_eq!(parsed_disk.finish, "length");
        assert_eq!(parsed_disk.cached_tokens, 10000);
        assert_eq!(parsed_disk.prefill_tokens, 0);
        assert!((parsed_disk.cache_hit_pct - 100.0).abs() < 0.01);
        assert!((parsed_disk.cache_restore_ms - 85.5).abs() < 0.01);

        let prefix_miss = "2026-11-19 22:02:00 [INFO] [http] request=r10 event=completed method=POST path=/v1/chat/completions status=200 duration_ms=9200.0 outcome=completed prompt_tokens=18000 prefill_tokens=18000 generated_tokens=90 finish=stop cache=miss cached_tokens=0 cache_restore_ms=0.0 queue_ms=0.1 ttft_ms=8100.0 prefill_tps=2200.0 decode_tps=38.0 batch_width=1 draft_accepted=40 draft_proposed=60 cache_miss_reason=prefix_changed common_prefix_tokens=420 nearest_checkpoint_tokens=15000";
        let parsed_miss = InferenceMetrics::parse_log_line(prefix_miss).expect("prefix miss metrics");
        assert_eq!(parsed_miss.cache, "miss");
        assert_eq!(parsed_miss.cached_tokens, 0);
        assert!((parsed_miss.cache_hit_pct - 0.0).abs() < 0.01);
        assert_eq!(parsed_miss.cache_miss_reason.as_deref(), Some("prefix_changed"));
        assert_eq!(parsed_miss.common_prefix_tokens, Some(420));
        assert_eq!(parsed_miss.nearest_checkpoint_tokens, Some(15000));
    }

    #[test]
    fn test_parse_active_and_completed_request_lines() {
        let received = "2026-11-19 22:05:00 [INFO] [http] request=r15 event=received method=POST path=/v1/chat/completions body_bytes=84320";
        let active = ActiveInferenceRequest::parse_received_line(received).expect("should parse received POST");
        assert_eq!(active.request_id, "r15");
        assert_eq!(active.method, "POST");
        assert_eq!(active.path, "/v1/chat/completions");
        assert_eq!(active.body_bytes, 84320);

        let get_models = "2026-11-19 22:05:00 [INFO] [http] request=r16 event=received method=GET path=/v1/models body_bytes=0";
        assert!(ActiveInferenceRequest::parse_received_line(get_models).is_none());

        let completed = "2026-11-19 22:05:03 [WARN] [http] request=r15 event=completed method=POST path=/v1/chat/completions status=400 duration_ms=1.2 outcome=completed error_code=invalid_messages";
        assert_eq!(parse_completed_request_id(completed).as_deref(), Some("r15"));
    }
}
