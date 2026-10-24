use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub speculative: String, // "mtp", "dflash2", "off"
    pub mtp_policy: String,  // "survival", "length"
    pub prompt_lookup: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_auto_open_gui")]
    pub auto_open_gui: bool,
    #[serde(default)]
    pub auto_start_server: bool,
}

fn default_auto_open_gui() -> bool {
    true
}

impl Default for GufoServerConfig {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        // 工作目錄下尋找 gufo.exe 的相對候選
        let gufo_candidates = [
            r"gufo.exe",
            r"build\release\gufo.exe",
            r"bin\gufo.exe",
        ];

        let default_gufo = gufo_candidates
            .iter()
            .find(|rel| cwd.join(rel).exists() || exe_dir.join(rel).exists())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if cwd.join(r"build\release\gufo.exe").exists() || exe_dir.join(r"build\release\gufo.exe").exists() {
                    r"build\release\gufo.exe".to_string()
                } else {
                    r"gufo.exe".to_string()
                }
            });

        // 工作目錄下尋找模型目錄的相對候選
        let model_dir_candidates = [
            r"models\Qwen3.8-Flash-Next-GGUF",
            r"models",
            r"",
        ];

        let chosen_model_dir = model_dir_candidates
            .iter()
            .find(|rel| {
                let test_path = if rel.is_empty() {
                    PathBuf::from("Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf")
                } else {
                    PathBuf::from(rel).join("Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf")
                };
                cwd.join(&test_path).exists() || exe_dir.join(&test_path).exists()
            })
            .unwrap_or(&model_dir_candidates[0]);

        let model_path = if chosen_model_dir.is_empty() {
            r"Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf".to_string()
        } else {
            format!(r"{}\Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf", chosen_model_dir)
        };

        let mtp_path = if chosen_model_dir.is_empty() {
            r"mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf".to_string()
        } else {
            format!(r"{}\mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf", chosen_model_dir)
        };

        let vision_path = if chosen_model_dir.is_empty() {
            r"mmproj-BF16.gguf".to_string()
        } else {
            format!(r"{}\mmproj-BF16.gguf", chosen_model_dir)
        };

        Self {
            gufo_path: default_gufo,
            model_path,
            mtp_path,
            vision_path,
            served_model_name: "flash-next".to_string(),
            host: "127.0.0.1".to_string(),
            port: 12223, // 預設使用用戶指定的 12223 port
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServerStateEnum {
    Stopped,
    Starting,
    Running,
    Error,
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

pub struct GufoManager {
    child: Arc<Mutex<Option<Child>>>,
    is_adopted: Arc<Mutex<bool>>,
    config: Arc<Mutex<GufoServerConfig>>,
    state: Arc<Mutex<ServerStateEnum>>,
    start_time: Arc<Mutex<Option<Instant>>>,
    error_message: Arc<Mutex<Option<String>>>,
    recent_logs: Arc<Mutex<VecDeque<String>>>,
    recent_metrics: Arc<Mutex<VecDeque<InferenceMetrics>>>,
}

impl GufoManager {
    pub fn new() -> Self {
        let initial_config = Self::load_saved_config().unwrap_or_default();
        Self {
            child: Arc::new(Mutex::new(None)),
            is_adopted: Arc::new(Mutex::new(false)),
            config: Arc::new(Mutex::new(initial_config)),
            state: Arc::new(Mutex::new(ServerStateEnum::Stopped)),
            start_time: Arc::new(Mutex::new(None)),
            error_message: Arc::new(Mutex::new(None)),
            recent_logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
            recent_metrics: Arc::new(Mutex::new(VecDeque::with_capacity(50))),
        }
    }

    fn get_config_file_path() -> PathBuf {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        exe_dir.join("gufo-studio.config.json")
    }

    pub fn load_saved_config() -> Option<GufoServerConfig> {
        let path = Self::get_config_file_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str::<GufoServerConfig>(&content) {
                    return Some(cfg);
                }
            }
        }
        None
    }

    pub async fn save_config(
        &self,
        app: Option<&AppHandle>,
        cfg: GufoServerConfig,
    ) -> Result<(), String> {
        *self.config.lock().await = cfg.clone();
        if let Some(app_handle) = app {
            let current_state = self.state.lock().await.clone();
            crate::tray::update_tray_status(app_handle, &current_state, cfg.port);
        }
        let path = Self::get_config_file_path();
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("無法寫入設定檔 {}: {}", path.display(), e))?;
        Ok(())
    }

    pub async fn get_config(&self) -> GufoServerConfig {
        self.config.lock().await.clone()
    }

    pub async fn get_status(&self, app: Option<&AppHandle>) -> ServerStatus {
        let cfg = self.config.lock().await.clone();

        // 主動檢查子進程是否已經意外終止
        let mut unexpected_exit: Option<String> = None;
        {
            let mut child_guard = self.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                if let Ok(Some(exit_status)) = child.try_wait() {
                    *child_guard = None;
                    unexpected_exit = Some(exit_status.to_string());
                }
            }
        }

        if let Some(exit_str) = unexpected_exit {
            let mut st = self.state.lock().await;
            if *st != ServerStateEnum::Stopped {
                *st = ServerStateEnum::Stopped;
                drop(st);
                *self.start_time.lock().await = None;
                *self.is_adopted.lock().await = false;
                let log_msg = format!(
                    "[系統] ⚠️ 偵測到 Gufo 推理伺服器已意外中止 ({})，狀態已自動重置為已停止。",
                    exit_str
                );
                self.add_log(log_msg.clone()).await;
                if let Some(app_handle) = app {
                    let _ = app_handle.emit("gufo-log", log_msg);
                    crate::tray::update_tray_status(app_handle, &ServerStateEnum::Stopped, cfg.port);
                    let _ = app_handle.emit("gufo-status-changed", "stopped");
                }
            }
        } else {
            let is_adopted = *self.is_adopted.lock().await;
            let current_state = self.state.lock().await.clone();
            if is_adopted && current_state == ServerStateEnum::Running {
                let port_alive = tokio::time::timeout(
                    std::time::Duration::from_millis(300),
                    tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port)),
                )
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);

                if !port_alive {
                    *self.is_adopted.lock().await = false;
                    *self.state.lock().await = ServerStateEnum::Stopped;
                    *self.start_time.lock().await = None;
                    let log_msg =
                        "[系統] ⚠️ 偵測到外部 Gufo 推理伺服器已中止，狀態已自動重置為已停止。"
                            .to_string();
                    self.add_log(log_msg.clone()).await;
                    if let Some(app_handle) = app {
                        let _ = app_handle.emit("gufo-log", log_msg);
                        crate::tray::update_tray_status(
                            app_handle,
                            &ServerStateEnum::Stopped,
                            cfg.port,
                        );
                        let _ = app_handle.emit("gufo-status-changed", "stopped");
                    }
                }
            }
        }

        let state = self.state.lock().await.clone();
        let pid = {
            let child_guard = self.child.lock().await;
            child_guard.as_ref().and_then(|c| c.id())
        };
        let uptime = if state == ServerStateEnum::Running {
            self.start_time
                .lock()
                .await
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0)
        } else {
            0
        };
        let err = self.error_message.lock().await.clone();

        ServerStatus {
            state,
            pid,
            port: cfg.port,
            host: cfg.host.clone(),
            served_model_name: cfg.served_model_name.clone(),
            endpoint: format!("http://{}:{}/v1", cfg.host, cfg.port),
            uptime_secs: uptime,
            error_message: err,
            api_key: if cfg.api_key.trim().is_empty() {
                None
            } else {
                Some(cfg.api_key.trim().to_string())
            },
        }
    }

    pub async fn get_logs(&self) -> Vec<String> {
        self.recent_logs.lock().await.iter().cloned().collect()
    }

    pub async fn get_metrics(&self) -> Vec<InferenceMetrics> {
        self.recent_metrics.lock().await.iter().cloned().collect()
    }

    pub fn resolve_path(path_str: &str) -> PathBuf {
        let p = std::path::Path::new(path_str);
        if p.is_absolute() && p.exists() {
            return p.to_path_buf();
        }
        if let Ok(cwd) = std::env::current_dir() {
            let candidate = cwd.join(p);
            if candidate.exists() {
                return candidate;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let candidate = parent.join(p);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            if p.is_relative() {
                return cwd.join(p);
            }
        }
        p.to_path_buf()
    }

    pub async fn sync_existing_server_on_startup(&self, app: AppHandle) {
        if self.check_health().await {
            let cfg = self.config.lock().await.clone();
            let mut st = self.state.lock().await;
            if *st == ServerStateEnum::Stopped {
                *st = ServerStateEnum::Running;
                drop(st);
                *self.is_adopted.lock().await = true;
                *self.start_time.lock().await = Some(Instant::now());
                *self.error_message.lock().await = None;

                let log_msg = format!(
                    "[系統] 偵測到 Port {} 已有運行的 Gufo 伺服器，已自動同步運行狀態。",
                    cfg.port
                );
                self.add_log(log_msg.clone()).await;
                let _ = app.emit("gufo-log", log_msg);
                crate::tray::update_tray_status(&app, &ServerStateEnum::Running, cfg.port);
                let _ = app.emit("gufo-status-changed", "running");

                self.spawn_lifecycle_monitor(
                    app,
                    cfg.host.clone(),
                    cfg.port,
                    cfg.api_key.trim().to_string(),
                );
            }
        }
    }

    fn spawn_lifecycle_monitor(
        &self,
        app: AppHandle,
        host: String,
        port: u16,
        api_key: String,
    ) {
        let state_clone = self.state.clone();
        let child_clone = self.child.clone();
        let is_adopted_clone = self.is_adopted.clone();
        let start_time_clone = self.start_time.clone();
        let error_msg_clone = self.error_message.clone();
        let logs_clone = self.recent_logs.clone();

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(800))
            .build()
            .unwrap_or_default();
        let health_url = format!("http://{}:{}/health", host, port);

        tokio::spawn(async move {
            // 第一階段：若處於 Starting，每 500ms 探測一次健康狀態，最多嘗試 240 次 (約 2 分鐘)
            for _ in 0..240 {
                {
                    let st = state_clone.lock().await;
                    if *st == ServerStateEnum::Running {
                        break;
                    }
                    if *st == ServerStateEnum::Stopped || *st == ServerStateEnum::Error {
                        return;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                {
                    let st = state_clone.lock().await;
                    if *st == ServerStateEnum::Running {
                        break;
                    }
                    if *st == ServerStateEnum::Stopped || *st == ServerStateEnum::Error {
                        return;
                    }
                }

                // 檢查進程是否在啟動期間意外退出
                {
                    let mut child_guard = child_clone.lock().await;
                    if let Some(ref mut child) = *child_guard {
                        if let Ok(Some(exit_status)) = child.try_wait() {
                            *child_guard = None;
                            drop(child_guard);
                            *start_time_clone.lock().await = None;
                            let err_msg = format!("Gufo 推理伺服器在啟動階段意外中止 ({})", exit_status);
                            *error_msg_clone.lock().await = Some(err_msg.clone());
                            *state_clone.lock().await = ServerStateEnum::Error;
                            let log_line = format!("[錯誤] {}", err_msg);
                            {
                                let mut logs = logs_clone.lock().await;
                                if logs.len() >= 1000 {
                                    logs.pop_front();
                                }
                                logs.push_back(log_line.clone());
                            }
                            let _ = app.emit("gufo-log", log_line);
                            crate::tray::update_tray_status(&app, &ServerStateEnum::Error, port);
                            let _ = app.emit("gufo-status-changed", "error");
                            return;
                        }
                    } else if !*is_adopted_clone.lock().await {
                        return;
                    }
                }

                // HTTP GET /health 探測
                let mut req = client.get(&health_url);
                if !api_key.is_empty() {
                    req = req.bearer_auth(&api_key);
                }

                if let Ok(resp) = req.send().await {
                    if resp.status().is_success() {
                        let mut st = state_clone.lock().await;
                        if *st == ServerStateEnum::Starting {
                            *st = ServerStateEnum::Running;
                            drop(st);
                            crate::tray::update_tray_status(&app, &ServerStateEnum::Running, port);
                            let _ = app.emit("gufo-status-changed", "running");
                        }
                        break;
                    }
                }
            }

            // 若超過嘗試時間仍處於 Starting，檢查進程是否仍在運行
            {
                let mut st = state_clone.lock().await;
                if *st == ServerStateEnum::Starting {
                    let mut child_guard = child_clone.lock().await;
                    if let Some(ref mut child) = *child_guard {
                        if let Ok(None) = child.try_wait() {
                            *st = ServerStateEnum::Running;
                            drop(child_guard);
                            drop(st);
                            crate::tray::update_tray_status(&app, &ServerStateEnum::Running, port);
                            let _ = app.emit("gufo-status-changed", "running");
                        } else {
                            *child_guard = None;
                            drop(child_guard);
                            *st = ServerStateEnum::Error;
                            drop(st);
                            *start_time_clone.lock().await = None;
                            crate::tray::update_tray_status(&app, &ServerStateEnum::Error, port);
                            let _ = app.emit("gufo-status-changed", "error");
                            return;
                        }
                    } else {
                        return;
                    }
                }
            }

            // 第二階段：持續監控運行中的伺服器，偵測意外崩潰或被外部終止
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

                {
                    let st = state_clone.lock().await;
                    if *st != ServerStateEnum::Running {
                        return;
                    }
                }

                let mut child_guard = child_clone.lock().await;
                if let Some(ref mut child) = *child_guard {
                    if let Ok(Some(exit_status)) = child.try_wait() {
                        *child_guard = None;
                        drop(child_guard);

                        let mut st = state_clone.lock().await;
                        if *st != ServerStateEnum::Stopped {
                            *st = ServerStateEnum::Stopped;
                            drop(st);
                            *start_time_clone.lock().await = None;
                            *is_adopted_clone.lock().await = false;

                            let log_msg = format!(
                                "[系統] ⚠️ 偵測到 Gufo 推理伺服器已意外中止 ({})，狀態已自動重置為已停止。",
                                exit_status
                            );
                            {
                                let mut logs = logs_clone.lock().await;
                                if logs.len() >= 1000 {
                                    logs.pop_front();
                                }
                                logs.push_back(log_msg.clone());
                            }
                            let _ = app.emit("gufo-log", log_msg);
                            crate::tray::update_tray_status(&app, &ServerStateEnum::Stopped, port);
                            let _ = app.emit("gufo-status-changed", "stopped");
                        }
                        return;
                    }
                } else {
                    drop(child_guard);
                    if *is_adopted_clone.lock().await {
                        let port_alive = tokio::time::timeout(
                            std::time::Duration::from_millis(500),
                            tokio::net::TcpStream::connect((host.as_str(), port)),
                        )
                        .await
                        .map(|r| r.is_ok())
                        .unwrap_or(false);

                        if !port_alive {
                            *is_adopted_clone.lock().await = false;
                            let mut st = state_clone.lock().await;
                            if *st != ServerStateEnum::Stopped {
                                *st = ServerStateEnum::Stopped;
                                drop(st);
                                *start_time_clone.lock().await = None;

                                let log_msg =
                                    "[系統] ⚠️ 偵測到外部 Gufo 推理伺服器已中止，狀態已自動重置為已停止。"
                                        .to_string();
                                {
                                    let mut logs = logs_clone.lock().await;
                                    if logs.len() >= 1000 {
                                        logs.pop_front();
                                    }
                                    logs.push_back(log_msg.clone());
                                }
                                let _ = app.emit("gufo-log", log_msg);
                                crate::tray::update_tray_status(
                                    &app,
                                    &ServerStateEnum::Stopped,
                                    port,
                                );
                                let _ = app.emit("gufo-status-changed", "stopped");
                            }
                            return;
                        }
                    } else {
                        return;
                    }
                }
            }
        });
    }

    pub async fn start(&self, app: AppHandle, new_config: Option<GufoServerConfig>) -> Result<ServerStatus, String> {
        // 先檢查伺服器是否已在運行中（若子進程已意外退出則先清理）
        {
            let mut child_guard = self.child.lock().await;
            if let Some(ref mut child) = *child_guard {
                if let Ok(Some(_)) = child.try_wait() {
                    *child_guard = None;
                } else {
                    return Err("伺服器已在運行中".to_string());
                }
            }
        }

        if let Some(cfg) = new_config {
            *self.config.lock().await = cfg;
        }
        let cfg = self.config.lock().await.clone();

        // 啟動前檢查：目標 host:port 是否已有活著的 Gufo 伺服器（例如先前實例啟動）
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(600))
            .build()
            .unwrap_or_default();
        let health_url = format!("http://{}:{}/health", cfg.host, cfg.port);
        let mut req = client.get(&health_url);
        if !cfg.api_key.trim().is_empty() {
            req = req.bearer_auth(cfg.api_key.trim());
        }

        if let Ok(resp) = req.send().await {
            if resp.status().is_success() {
                self.add_log(format!(
                    "[系統] 偵測到 Port {} 已有運行的 Gufo 伺服器，已自動接管該服務，避免重複啟動耗損顯存。",
                    cfg.port
                ))
                .await;
                *self.state.lock().await = ServerStateEnum::Running;
                *self.is_adopted.lock().await = true;
                *self.start_time.lock().await = Some(Instant::now());
                *self.error_message.lock().await = None;
                crate::tray::update_tray_status(&app, &ServerStateEnum::Running, cfg.port);
                let _ = app.emit("gufo-status-changed", "running");

                self.spawn_lifecycle_monitor(
                    app.clone(),
                    cfg.host.clone(),
                    cfg.port,
                    cfg.api_key.trim().to_string(),
                );
                return Ok(self.get_status(Some(&app)).await);
            }
        }

        // 若不是合法的 Gufo 伺服器但該 Port 已被佔用，提早阻擋避免衝突
        if tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port)).await.is_ok() {
            let err_msg = format!(
                "Port {} 目前已被佔用但無法通過健康檢查，請確認是否有異常殘留程序或更換 Port",
                cfg.port
            );
            self.add_log(format!("[錯誤] {}", err_msg)).await;
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(err_msg.clone());
            crate::tray::update_tray_status(&app, &ServerStateEnum::Error, cfg.port);
            let _ = app.emit("gufo-status-changed", "error");
            return Err(err_msg);
        }

        let resolved_gufo = Self::resolve_path(&cfg.gufo_path);
        let resolved_model = Self::resolve_path(&cfg.model_path);

        // 檢查執行檔是否存在
        if !resolved_gufo.exists() {
            let msg = format!("找不到 Gufo 主程式檔案：{}", cfg.gufo_path);
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(msg.clone());
            crate::tray::update_tray_status(&app, &ServerStateEnum::Error, cfg.port);
            let _ = app.emit("gufo-status-changed", "error");
            return Err(msg);
        }

        // 檢查模型檔案是否存在
        if !resolved_model.exists() {
            let msg = format!("找不到模型主檔：{}", cfg.model_path);
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(msg.clone());
            crate::tray::update_tray_status(&app, &ServerStateEnum::Error, cfg.port);
            let _ = app.emit("gufo-status-changed", "error");
            return Err(msg);
        }

        *self.state.lock().await = ServerStateEnum::Starting;
        *self.is_adopted.lock().await = false;
        *self.error_message.lock().await = None;
        *self.start_time.lock().await = None;
        crate::tray::update_tray_status(&app, &ServerStateEnum::Starting, cfg.port);
        let _ = app.emit("gufo-status-changed", "starting");

        // 組裝指令參數
        let mut args: Vec<String> = vec![
            "serve".to_string(),
            "--host".to_string(),
            cfg.host.clone(),
            "--port".to_string(),
            cfg.port.to_string(),
            "--sessions".to_string(),
            cfg.sessions.to_string(),
            "--max-request-bytes".to_string(),
            cfg.max_request_bytes.to_string(),
            "llm".to_string(),
            "--model".to_string(),
            resolved_model.to_string_lossy().to_string(),
            "--served-model-name".to_string(),
            cfg.served_model_name.clone(),
            "--context".to_string(),
            cfg.context.to_string(),
            "--max-output-bytes".to_string(),
            cfg.max_output_bytes.to_string(),
            "--think".to_string(),
            cfg.think.clone(),
            "--temperature".to_string(),
            cfg.temperature.to_string(),
        ];

        if !cfg.api_key.trim().is_empty() {
            args.push("--api-key".to_string());
            args.push(cfg.api_key.trim().to_string());
        }

        if cfg.speculative == "mtp" && !cfg.mtp_path.is_empty() {
            let resolved_mtp = Self::resolve_path(&cfg.mtp_path);
            if resolved_mtp.exists() {
                args.push("--speculative".to_string());
                args.push("mtp".to_string());
                args.push("--mtp-model".to_string());
                args.push(resolved_mtp.to_string_lossy().to_string());
                if !cfg.mtp_policy.is_empty() {
                    args.push("--mtp-policy".to_string());
                    args.push(cfg.mtp_policy.clone());
                }
            }
        }

        if cfg.prompt_lookup {
            args.push("--prompt-lookup".to_string());
        }

        if !cfg.vision_path.is_empty() {
            let resolved_vision = Self::resolve_path(&cfg.vision_path);
            if resolved_vision.exists() {
                args.push("--mmproj".to_string());
                args.push(resolved_vision.to_string_lossy().to_string());
            }
        }

        let cmd_display = format!("{} {}", resolved_gufo.display(), args.join(" "));
        self.add_log(format!("[系統] 準備啟動：{}", cmd_display)).await;

        let mut command = Command::new(&resolved_gufo);
        if let Ok(cwd) = std::env::current_dir() {
            command.current_dir(cwd);
        }
        command
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // 在 Windows 隱藏命令列黑視窗
        #[cfg(target_os = "windows")]
        {
            // CREATE_NO_WINDOW = 0x08000000
            command.creation_flags(0x08000000);
        }

        let mut spawned = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err = format!("無法啟動 Gufo 程序: {}", e);
                *self.state.lock().await = ServerStateEnum::Error;
                *self.error_message.lock().await = Some(err.clone());
                crate::tray::update_tray_status(&app, &ServerStateEnum::Error, cfg.port);
                let _ = app.emit("gufo-status-changed", "error");
                return Err(err);
            }
        };

        let stdout = spawned.stdout.take();
        let stderr = spawned.stderr.take();

        // 存入 spawned 進程並立即釋放鎖，防止後續 self.get_status() 發生死鎖
        {
            let mut child_guard = self.child.lock().await;
            *child_guard = Some(spawned);
        }
        *self.start_time.lock().await = Some(Instant::now());

        let port = cfg.port;

        // 啟動 stdout 讀取任務
        let logs_clone1 = self.recent_logs.clone();
        let metrics_clone1 = self.recent_metrics.clone();
        let state_clone1 = self.state.clone();
        let app_clone1 = app.clone();
        if let Some(stdout) = stdout {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut logs = logs_clone1.lock().await;
                    if logs.len() >= 1000 {
                        logs.pop_front();
                    }
                    logs.push_back(line.clone());
                    let _ = app_clone1.emit("gufo-log", line.clone());

                    // 解析推論度量指標（token/s、prefill_tps、ttft_ms 等）
                    if let Some(metrics) = InferenceMetrics::parse_log_line(&line) {
                        let mut m = metrics_clone1.lock().await;
                        if m.len() >= 50 {
                            m.pop_front();
                        }
                        m.push_back(metrics.clone());
                        let _ = app_clone1.emit("gufo-metrics", metrics);
                    }

                    let line_lower = line.to_lowercase();
                    if line_lower.contains("listening")
                        || line_lower.contains("serving")
                        || line_lower.contains("http server")
                    {
                        let mut st = state_clone1.lock().await;
                        if *st == ServerStateEnum::Starting {
                            *st = ServerStateEnum::Running;
                            drop(st);
                            crate::tray::update_tray_status(
                                &app_clone1,
                                &ServerStateEnum::Running,
                                port,
                            );
                            let _ = app_clone1.emit("gufo-status-changed", "running");
                        }
                    }
                }
            });
        }

        // 啟動 stderr 讀取任務
        let logs_clone2 = self.recent_logs.clone();
        let metrics_clone2 = self.recent_metrics.clone();
        let state_clone2 = self.state.clone();
        let app_clone2 = app.clone();
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let mut logs = logs_clone2.lock().await;
                    if logs.len() >= 1000 {
                        logs.pop_front();
                    }
                    logs.push_back(line.clone());
                    let _ = app_clone2.emit("gufo-log", line.clone());

                    // 解析推論度量指標（token/s、prefill_tps、ttft_ms 等）
                    if let Some(metrics) = InferenceMetrics::parse_log_line(&line) {
                        let mut m = metrics_clone2.lock().await;
                        if m.len() >= 50 {
                            m.pop_front();
                        }
                        m.push_back(metrics.clone());
                        let _ = app_clone2.emit("gufo-metrics", metrics);
                    }

                    let line_lower = line.to_lowercase();
                    if line_lower.contains("listening")
                        || line_lower.contains("serving")
                        || line_lower.contains("http server")
                    {
                        let mut st = state_clone2.lock().await;
                        if *st == ServerStateEnum::Starting {
                            *st = ServerStateEnum::Running;
                            drop(st);
                            crate::tray::update_tray_status(
                                &app_clone2,
                                &ServerStateEnum::Running,
                                port,
                            );
                            let _ = app_clone2.emit("gufo-status-changed", "running");
                        }
                    }
                }
            });
        }

        // 啟動全生命週期監控（包含啟動健康檢查與運行期間意外中止偵測）
        self.spawn_lifecycle_monitor(
            app.clone(),
            cfg.host.clone(),
            cfg.port,
            cfg.api_key.trim().to_string(),
        );

        Ok(self.get_status(Some(&app)).await)
    }

    pub async fn stop(&self, app: Option<&AppHandle>) -> Result<ServerStatus, String> {
        *self.state.lock().await = ServerStateEnum::Stopped;
        *self.is_adopted.lock().await = false;
        *self.start_time.lock().await = None;
        *self.error_message.lock().await = None;

        let maybe_child = {
            let mut child_guard = self.child.lock().await;
            child_guard.take()
        };

        if let Some(mut child) = maybe_child {
            self.add_log("[系統] 正在停止 Gufo 推理引擎...".to_string()).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            self.add_log("[系統] Gufo 子進程已終止。".to_string()).await;
        }

        // 清理背景可能殘留的孤兒 gufo 進程（確保 Strix Halo 96GB 顯存完全釋放）
        let killed_count = Self::kill_orphan_gufo_processes().await;
        if killed_count > 0 {
            self.add_log(format!(
                "[系統] 已清理背景殘留的 {} 個 gufo 進程，GPU 顯存已完全釋放。",
                killed_count
            ))
            .await;
        } else {
            self.add_log("[系統] GPU 顯存已完全釋放。".to_string()).await;
        }

        let cfg = self.config.lock().await.clone();
        if let Some(app_handle) = app {
            crate::tray::update_tray_status(app_handle, &ServerStateEnum::Stopped, cfg.port);
            let _ = app_handle.emit("gufo-status-changed", "stopped");
        }

        Ok(self.get_status(app).await)
    }

    pub async fn kill_orphan_gufo_processes() -> usize {
        #[cfg(target_os = "windows")]
        {
            tokio::task::spawn_blocking(|| {
                use std::os::windows::process::CommandExt;
                let mut check_cmd = std::process::Command::new("tasklist");
                check_cmd.args(&["/FI", "IMAGENAME eq gufo.exe", "/NH"]);
                check_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
                if let Ok(output) = check_cmd.output() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    if text.contains("gufo.exe") {
                        let mut kill_cmd = std::process::Command::new("taskkill");
                        kill_cmd.args(&["/F", "/IM", "gufo.exe"]);
                        kill_cmd.creation_flags(0x08000000);
                        let _ = kill_cmd.output();
                        return 1;
                    }
                }
                0
            })
            .await
            .unwrap_or(0)
        }
        #[cfg(not(target_os = "windows"))]
        {
            tokio::task::spawn_blocking(|| {
                let mut kill_cmd = std::process::Command::new("pkill");
                kill_cmd.args(&["-9", "-x", "gufo"]);
                if let Ok(status) = kill_cmd.status() {
                    if status.success() {
                        return 1;
                    }
                }
                0
            })
            .await
            .unwrap_or(0)
        }
    }

    pub async fn check_health(&self) -> bool {
        let cfg = self.config.lock().await.clone();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(600))
            .build()
            .unwrap_or_default();
        let health_url = format!("http://{}:{}/health", cfg.host, cfg.port);
        let mut req = client.get(&health_url);
        if !cfg.api_key.trim().is_empty() {
            req = req.bearer_auth(cfg.api_key.trim());
        }
        if let Ok(resp) = req.send().await {
            resp.status().is_success()
        } else {
            false
        }
    }

    async fn add_log(&self, line: String) {
        let mut logs = self.recent_logs.lock().await;
        if logs.len() >= 1000 {
            logs.pop_front();
        }
        logs.push_back(line);
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

        // 預設路徑應為相對工作目錄，不應寫死 D:\Agents 或 D:\Models
        assert!(!config.gufo_path.starts_with(r"D:\Agents\gufo"));
        assert!(!config.model_path.starts_with(r"D:\Models\GGUF"));
    }

    #[tokio::test]
    async fn test_api_key_configuration() {
        let mut config = GufoServerConfig::default();
        config.api_key = "sk-gufo-secret-key-123".to_string();
        assert_eq!(config.api_key, "sk-gufo-secret-key-123");
    }

    #[tokio::test]
    async fn test_manager_initial_status() {
        let manager = GufoManager::new();
        let status = manager.get_status(None).await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.port, 12223);
        assert_eq!(status.pid, None);
        assert_eq!(status.endpoint, "http://127.0.0.1:12223/v1");

        // 測試 stop 不會因為 Mutex 造成死鎖
        let stop_status = manager.stop(None).await.unwrap();
        assert_eq!(stop_status.state, ServerStateEnum::Stopped);
    }

    #[tokio::test]
    async fn test_unexpected_process_exit_resets_status() {
        let manager = GufoManager::new();
        // 模擬啟動一個立即退出的子進程，且狀態為 Running
        #[cfg(target_os = "windows")]
        let child = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
        #[cfg(not(target_os = "windows"))]
        let child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();

        *manager.child.lock().await = Some(child);
        *manager.state.lock().await = ServerStateEnum::Running;
        *manager.start_time.lock().await = Some(Instant::now());

        // 等待子進程退出
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // 查詢狀態時應自動偵測子進程已不存在，並重置為 Stopped
        let status = manager.get_status(None).await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.pid, None);
        assert_eq!(status.uptime_secs, 0);
    }

    #[tokio::test]
    async fn test_resolve_relative_path() {
        let p = GufoManager::resolve_path("Cargo.toml");
        assert!(p.is_relative() || p.is_absolute());
    }

    #[test]
    fn test_parse_metrics_log_line() {
        let sample = "2026-10-24 14:00:00 [INFO] [http] request=r2 event=completed method=POST path=/v1/chat/completions status=200 duration_ms=4520.5 prompt_tokens=512 prefill_tokens=512 generated_tokens=159 ttft_ms=12.5 prefill_tps=1240.5 decode_tps=31.8 draft_accepted=85 draft_proposed=100 acceptance_pct=85.0";
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

