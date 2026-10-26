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

use crate::process_control;

const MAX_LOG_LINES: usize = 1000;
const MAX_METRICS: usize = 50;
/// 正常關閉 (Ctrl+C / SIGINT) 的等待上限；Gufo 會在此期間排空磁碟快取寫入。
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(15);
/// 模型載入超過此時間仍未就緒時，於日誌提示可能的原因 (不會中斷載入)。
const SLOW_LOAD_HINT_AFTER: Duration = Duration::from_secs(180);

/// 缺少的欄位一律採用 `Default`，讓舊版設定檔可直接升級。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GufoServerConfig {
    pub gufo_path: String,
    pub model_path: String,
    pub mtp_path: String,
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
    pub speculative: String, // "mtp", "off"
    pub mtp_policy: String,  // "survival", "length"
    pub prompt_lookup: bool,
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
            vision_path: in_model_dir("mmproj-BF16.gguf"),
            served_model_name: "flash-next".to_string(),
            host: "127.0.0.1".to_string(),
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
            api_key: String::new(),
            auto_open_gui: true,
            auto_start_server: false,
            max_tokens: 32768,
            draft_tokens: 0,
            mtp_draft_vocab: "full".to_string(),
            prefill_chunk: 0,
            cache_disk: false,
            cache_disk_dir: "gufo-cache".to_string(),
            cache_disk_bytes: 0,
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
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
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
                    _ => {}
                }
            }
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

/// 組裝 `gufo serve ... llm ...` 參數。純函式，方便測試。
pub fn build_args(
    cfg: &GufoServerConfig,
    model: &Path,
    mtp: Option<&Path>,
    vision: Option<&Path>,
    cache_dir: Option<&Path>,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

    push(&["serve", "--host", &cfg.host, "--port", &cfg.port.to_string()]);
    push(&["--sessions", &cfg.sessions.to_string()]);
    push(&["--max-request-bytes", &cfg.max_request_bytes.to_string()]);
    let api_key = cfg.api_key.trim();
    if !api_key.is_empty() {
        push(&["--api-key", api_key]);
    }

    push(&["llm", "--model", &model.to_string_lossy()]);
    push(&["--served-model-name", &cfg.served_model_name]);
    push(&["--context", &cfg.context.to_string()]);
    push(&["--max-output-bytes", &cfg.max_output_bytes.to_string()]);
    push(&["--think", &cfg.think]);
    push(&["--temperature", &cfg.temperature.to_string()]);
    if cfg.max_tokens > 0 {
        push(&["--max-tokens", &cfg.max_tokens.to_string()]);
    }

    if let Some(mtp) = mtp {
        push(&["--speculative", "mtp", "--mtp-model", &mtp.to_string_lossy()]);
        if !cfg.mtp_policy.is_empty() {
            push(&["--mtp-policy", &cfg.mtp_policy]);
        }
        if cfg.draft_tokens > 0 {
            push(&["--draft-tokens", &cfg.draft_tokens.to_string()]);
        }
        if cfg.mtp_draft_vocab == "latin" {
            push(&["--mtp-draft-vocab", "latin"]);
        }
    }

    if cfg.prompt_lookup {
        push(&["--prompt-lookup"]);
    }
    if cfg.prefill_chunk > 0 {
        push(&["--prefill-chunk", &cfg.prefill_chunk.to_string()]);
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
}

impl Runtime {
    /// 子程序結束時更新狀態，回傳 (新狀態, 日誌訊息)；若是預期中的停止則回傳 None。
    fn on_child_exited(&mut self, status: ExitStatus) -> Option<(ServerStateEnum, String)> {
        self.child = None;
        self.start_time = None;
        self.adopted = false;
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

    /// 若仍屬於同一世代且處於 Starting，轉為 Running 並回傳載入耗時。
    async fn mark_running_if_starting(&self, generation: u64, app: Option<&AppHandle>, port: u16) -> Option<Duration> {
        let elapsed = {
            let mut rt = self.runtime.lock().await;
            if self.current_generation() != generation || rt.state != ServerStateEnum::Starting {
                return None;
            }
            rt.state = ServerStateEnum::Running;
            rt.start_time.get_or_insert_with(Instant::now).elapsed()
        };
        notify_state(app, &ServerStateEnum::Running, port);
        self.log(
            app,
            format!("[系統] ✅ Gufo 推理伺服器已就緒 (模型載入耗時 {:.1} 秒)。", elapsed.as_secs_f64()),
        )
        .await;
        Some(elapsed)
    }
}

fn notify_state(app: Option<&AppHandle>, state: &ServerStateEnum, port: u16) {
    if let Some(app) = app {
        crate::tray::update_tray_status(app, state, port);
        let _ = app.emit("gufo-status-changed", state.as_str());
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
    let base = format!("http://{}:{}", host, port);
    match probe_status(client, &format!("{}/ready", base), api_key).await {
        Some(s) if s.is_success() => true,
        Some(reqwest::StatusCode::NOT_FOUND) => {
            matches!(probe_status(client, &format!("{}/health", base), api_key).await, Some(s) if s.is_success())
        }
        _ => false,
    }
}

async fn port_open(host: &str, port: u16, timeout: Duration) -> bool {
    tokio::time::timeout(timeout, tokio::net::TcpStream::connect((host, port)))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// 逐行讀取子程序輸出。使用 lossy 解碼，避免非 UTF-8 輸出 (例如系統錯誤訊息)
/// 讓讀取中斷進而塞滿管線、卡住 Gufo。
fn spawn_output_reader<R>(shared: Arc<Shared>, app: AppHandle, generation: u64, port: u16, stream: R)
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
            if let Some(metrics) = InferenceMetrics::parse_log_line(&line) {
                shared.push_metrics(Some(&app), metrics).await;
            }
            let ready = is_listening_line(&line);
            shared.log(Some(&app), line).await;
            if ready {
                shared.mark_running_if_starting(generation, Some(&app), port).await;
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
                            notify_state(Some(&app), &state, port);
                        }
                        return;
                    }
                } else if !rt.adopted {
                    return;
                }
            }

            if probe_ready(&client, &host, port, &api_key).await {
                shared.mark_running_if_starting(generation, Some(&app), port).await;
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
                        notify_state(Some(&app), &state, port);
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
            drop(rt);
            shared
                .log(Some(&app), "[系統] ⚠️ 偵測到外部 Gufo 推理伺服器已中止，狀態已自動重置為已停止。")
                .await;
            notify_state(Some(&app), &ServerStateEnum::Stopped, port);
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

    fn get_config_file_path() -> PathBuf {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        exe_dir.join("gufo-studio.config.json")
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
            crate::tray::update_tray_status(app_handle, &current_state, cfg.port);
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

        let (status, transition) = {
            let mut rt = self.shared.runtime.lock().await;
            // 主動檢查子程序是否已意外終止 (監控任務也會偵測，此處讓輪詢立即反映)
            let exited = rt.child.as_mut().and_then(|c| c.try_wait().ok().flatten());
            let transition = exited.and_then(|status| rt.on_child_exited(status));
            let uptime = match (&rt.state, rt.start_time) {
                (ServerStateEnum::Running, Some(t)) => t.elapsed().as_secs(),
                _ => 0,
            };
            let status = ServerStatus {
                state: rt.state.clone(),
                pid: rt.child.as_ref().and_then(|c| c.id()),
                port: cfg.port,
                host: cfg.host.clone(),
                served_model_name: cfg.served_model_name.clone(),
                endpoint: format!("http://{}:{}/v1", cfg.host, cfg.port),
                uptime_secs: uptime,
                error_message: rt.error_message.clone(),
                api_key: Some(cfg.api_key.trim().to_string()).filter(|k| !k.is_empty()),
            };
            (status, transition)
        };

        if let Some((state, msg)) = transition {
            self.shared.log(app, msg).await;
            notify_state(app, &state, cfg.port);
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

    /// 將指定 Port 上已就緒的 Gufo 伺服器標記為「已接管」並開始監控。
    async fn adopt_running_server(&self, app: AppHandle, cfg: &GufoServerConfig, message: String) {
        let generation = {
            let mut rt = self.shared.runtime.lock().await;
            let generation = self.shared.next_generation();
            rt.state = ServerStateEnum::Running;
            rt.adopted = true;
            rt.start_time = Some(Instant::now());
            rt.error_message = None;
            generation
        };
        self.shared.log(Some(&app), message).await;
        notify_state(Some(&app), &ServerStateEnum::Running, cfg.port);
        spawn_lifecycle_monitor(
            self.shared.clone(),
            app,
            generation,
            cfg.host.clone(),
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

    async fn fail(&self, app: &AppHandle, port: u16, msg: String) -> Result<ServerStatus, String> {
        {
            let mut rt = self.shared.runtime.lock().await;
            self.shared.next_generation();
            rt.state = ServerStateEnum::Error;
            rt.error_message = Some(msg.clone());
            rt.start_time = None;
        }
        self.shared.log(Some(app), format!("[錯誤] {}", msg)).await;
        notify_state(Some(app), &ServerStateEnum::Error, port);
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
            return self.fail(&app, port, msg).await;
        }

        let resolved_gufo = Self::resolve_path(&cfg.gufo_path);
        if !resolved_gufo.exists() {
            return self.fail(&app, port, format!("找不到 Gufo 主程式檔案：{}", cfg.gufo_path)).await;
        }
        let resolved_model = Self::resolve_path(&cfg.model_path);
        if !resolved_model.exists() {
            return self.fail(&app, port, format!("找不到模型主檔：{}", cfg.model_path)).await;
        }

        let mut warnings = Vec::new();
        let mtp = if cfg.speculative == "mtp" && !cfg.mtp_path.trim().is_empty() {
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
                return self.fail(&app, port, format!("無法建立磁碟快取目錄 {}：{}", dir.display(), e)).await;
            }
            Some(dir)
        } else {
            None
        };

        let generation = {
            let mut rt = self.shared.runtime.lock().await;
            let generation = self.shared.next_generation();
            rt.state = ServerStateEnum::Starting;
            rt.adopted = false;
            rt.error_message = None;
            rt.start_time = None;
            generation
        };
        notify_state(Some(&app), &ServerStateEnum::Starting, port);
        for w in warnings {
            self.shared.log(Some(&app), w).await;
        }

        let args = build_args(&cfg, &resolved_model, mtp.as_deref(), vision.as_deref(), cache_dir.as_deref());
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
            Err(e) => return self.fail(&app, port, format!("無法啟動 Gufo 程序: {}", e)).await,
        };
        let stdout = spawned.stdout.take();
        let stderr = spawned.stderr.take();
        {
            let mut rt = self.shared.runtime.lock().await;
            rt.child = Some(spawned);
            rt.start_time = Some(Instant::now());
        }

        if let Some(stdout) = stdout {
            spawn_output_reader(self.shared.clone(), app.clone(), generation, port, stdout);
        }
        if let Some(stderr) = stderr {
            spawn_output_reader(self.shared.clone(), app.clone(), generation, port, stderr);
        }
        spawn_lifecycle_monitor(self.shared.clone(), app.clone(), generation, cfg.host.clone(), port, api_key);

        Ok(self.get_status(Some(&app)).await)
    }

    /// 停止伺服器。只會終止本程式啟動的子程序，或在設定 Port 上監聽、且確認為 Gufo 的已接管程序；
    /// 不會以映像名稱終止其他 Gufo 實例。
    pub async fn stop(&self, app: Option<&AppHandle>) -> Result<ServerStatus, String> {
        let _op = self.shared.op_lock.lock().await;
        let (child, adopted) = {
            let mut rt = self.shared.runtime.lock().await;
            self.shared.next_generation();
            let child = rt.child.take();
            let adopted = rt.adopted;
            rt.state = ServerStateEnum::Stopped;
            rt.adopted = false;
            rt.start_time = None;
            rt.error_message = None;
            (child, adopted)
        };
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

        notify_state(app, &ServerStateEnum::Stopped, cfg.port);
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
        assert_eq!(config.host, "127.0.0.1");
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
        assert!(!config.cache_disk);

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
        assert_eq!(cfg.cache_disk_dir, "gufo-cache");
    }

    #[test]
    fn test_build_args_includes_optional_flags_and_redacts_key() {
        let mut cfg = GufoServerConfig::default();
        cfg.api_key = "sk-gufo-secret".to_string();
        cfg.draft_tokens = 5;
        cfg.mtp_draft_vocab = "latin".to_string();
        cfg.prefill_chunk = 1024;
        cfg.cache_disk_bytes = 1 << 30;
        let args = build_args(
            &cfg,
            Path::new("model.gguf"),
            Some(Path::new("mtp.gguf")),
            None,
            Some(Path::new("cache dir")),
        );
        let joined = args.join(" ");
        let llm = args.iter().position(|a| a == "llm").unwrap();
        let key = args.iter().position(|a| a == "--api-key").unwrap();
        assert!(key < llm, "server options must precede the llm subcommand");
        for expected in [
            "--max-tokens 32768",
            "--speculative mtp --mtp-model mtp.gguf",
            "--draft-tokens 5",
            "--mtp-draft-vocab latin",
            "--prefill-chunk 1024",
            "--cache-disk cache dir",
            "--cache-disk-bytes 1073741824",
            "--prompt-lookup",
        ] {
            assert!(joined.contains(expected), "missing `{}` in `{}`", expected, joined);
        }
        assert!(!joined.contains("--mmproj"));

        let shown = redacted_command(Path::new("gufo.exe"), &args);
        assert!(!shown.contains("sk-gufo-secret"));
        assert!(shown.contains("--api-key ******"));
        assert!(shown.contains("\"cache dir\""));
    }

    #[test]
    fn test_build_args_omits_unset_options() {
        let mut cfg = GufoServerConfig::default();
        cfg.max_tokens = 0;
        let args = build_args(&cfg, Path::new("model.gguf"), None, None, None).join(" ");
        for absent in ["--api-key", "--max-tokens", "--speculative", "--draft-tokens", "--mtp-draft-vocab", "--prefill-chunk", "--cache-disk"] {
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
        assert_eq!(status.port, 12223);
        assert_eq!(status.pid, None);
        assert_eq!(status.endpoint, "http://127.0.0.1:12223/v1");

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
        assert_eq!(parsed.generated_tokens, 159);
        assert!((parsed.prefill_tps - 1240.5).abs() < 0.01);
        assert!((parsed.decode_tps - 31.8).abs() < 0.01);
        assert!((parsed.ttft_ms - 12.5).abs() < 0.01);
        assert_eq!(parsed.draft_accepted, 85);
        assert_eq!(parsed.draft_proposed, 100);
        assert_eq!(parsed.acceptance_pct, Some(85.0));
    }
}
