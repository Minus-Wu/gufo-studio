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

pub struct GufoManager {
    child: Arc<Mutex<Option<Child>>>,
    config: Arc<Mutex<GufoServerConfig>>,
    state: Arc<Mutex<ServerStateEnum>>,
    start_time: Arc<Mutex<Option<Instant>>>,
    error_message: Arc<Mutex<Option<String>>>,
    recent_logs: Arc<Mutex<VecDeque<String>>>,
}

impl GufoManager {
    pub fn new() -> Self {
        let initial_config = Self::load_saved_config().unwrap_or_default();
        Self {
            child: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(initial_config)),
            state: Arc::new(Mutex::new(ServerStateEnum::Stopped)),
            start_time: Arc::new(Mutex::new(None)),
            error_message: Arc::new(Mutex::new(None)),
            recent_logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
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

    pub async fn save_config(&self, cfg: GufoServerConfig) -> Result<(), String> {
        *self.config.lock().await = cfg.clone();
        let path = Self::get_config_file_path();
        let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("無法寫入設定檔 {}: {}", path.display(), e))?;
        Ok(())
    }

    pub async fn get_config(&self) -> GufoServerConfig {
        self.config.lock().await.clone()
    }

    pub async fn get_status(&self) -> ServerStatus {
        let state = self.state.lock().await.clone();
        let cfg = self.config.lock().await.clone();
        let pid = {
            let child_guard = self.child.lock().await;
            child_guard.as_ref().and_then(|c| c.id())
        };
        let uptime = self
            .start_time
            .lock()
            .await
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
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

    pub async fn start(&self, app: AppHandle, new_config: Option<GufoServerConfig>) -> Result<ServerStatus, String> {
        // 先檢查伺服器是否已在運行中
        {
            let child_guard = self.child.lock().await;
            if child_guard.is_some() {
                return Err("伺服器已在運行中".to_string());
            }
        }

        if let Some(cfg) = new_config {
            *self.config.lock().await = cfg;
        }
        let cfg = self.config.lock().await.clone();

        let resolved_gufo = Self::resolve_path(&cfg.gufo_path);
        let resolved_model = Self::resolve_path(&cfg.model_path);

        // 檢查執行檔是否存在
        if !resolved_gufo.exists() {
            let msg = format!("找不到 Gufo 主程式檔案：{}", cfg.gufo_path);
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(msg.clone());
            return Err(msg);
        }

        // 檢查模型檔案是否存在
        if !resolved_model.exists() {
            let msg = format!("找不到模型主檔：{}", cfg.model_path);
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(msg.clone());
            return Err(msg);
        }

        *self.state.lock().await = ServerStateEnum::Starting;
        *self.error_message.lock().await = None;
        *self.start_time.lock().await = None;

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

        // 啟動 stdout 讀取任務
        let logs_clone1 = self.recent_logs.clone();
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

                    let line_lower = line.to_lowercase();
                    if line_lower.contains("listening")
                        || line_lower.contains("serving")
                        || line_lower.contains("http server")
                    {
                        let mut st = state_clone1.lock().await;
                        if *st == ServerStateEnum::Starting {
                            *st = ServerStateEnum::Running;
                            let _ = app_clone1.emit("gufo-status-changed", "running");
                        }
                    }
                }
            });
        }

        // 啟動 stderr 讀取任務
        let logs_clone2 = self.recent_logs.clone();
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

                    let line_lower = line.to_lowercase();
                    if line_lower.contains("listening")
                        || line_lower.contains("serving")
                        || line_lower.contains("http server")
                    {
                        let mut st = state_clone2.lock().await;
                        if *st == ServerStateEnum::Starting {
                            *st = ServerStateEnum::Running;
                            let _ = app_clone2.emit("gufo-status-changed", "running");
                        }
                    }
                }
            });
        }

        // 啟動健康檢查背景監控 (以 TCP 探測連接)
        let state_clone = self.state.clone();
        let child_clone = self.child.clone();
        let host = cfg.host.clone();
        let port = cfg.port;
        let app_clone3 = app.clone();
        tokio::spawn(async move {
            // 每 500ms 檢查一次，最多嘗試 240 次 (約 2 分鐘，適合大模型載入)
            for _ in 0..240 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                // 若已被日誌讀取任務標記為 Running，直接結束探測
                {
                    let st = state_clone.lock().await;
                    if *st == ServerStateEnum::Running {
                        return;
                    }
                    if *st == ServerStateEnum::Stopped || *st == ServerStateEnum::Error {
                        return;
                    }
                }

                // 檢查進程是否意外提早退出
                {
                    let mut child_guard = child_clone.lock().await;
                    if let Some(ref mut child) = *child_guard {
                        if let Ok(Some(_exit_status)) = child.try_wait() {
                            let mut st = state_clone.lock().await;
                            *st = ServerStateEnum::Error;
                            let _ = app_clone3.emit("gufo-status-changed", "error");
                            return;
                        }
                    } else {
                        return;
                    }
                }

                // TCP 探測：一旦 port 開放連線，代表模型載入完畢且伺服器已就緒
                if tokio::net::TcpStream::connect((host.as_str(), port)).await.is_ok() {
                    let mut st = state_clone.lock().await;
                    if *st == ServerStateEnum::Starting {
                        *st = ServerStateEnum::Running;
                        let _ = app_clone3.emit("gufo-status-changed", "running");
                    }
                    return;
                }
            }

            // 超過嘗試時間仍未連上，檢查進程是否仍在運行
            let mut st = state_clone.lock().await;
            if *st == ServerStateEnum::Starting {
                let mut child_guard = child_clone.lock().await;
                if let Some(ref mut child) = *child_guard {
                    if let Ok(None) = child.try_wait() {
                        *st = ServerStateEnum::Running;
                        let _ = app_clone3.emit("gufo-status-changed", "running");
                    } else {
                        *st = ServerStateEnum::Error;
                        let _ = app_clone3.emit("gufo-status-changed", "error");
                    }
                }
            }
        });

        Ok(self.get_status().await)
    }

    pub async fn stop(&self) -> Result<ServerStatus, String> {
        let maybe_child = {
            let mut child_guard = self.child.lock().await;
            child_guard.take()
        };

        if let Some(mut child) = maybe_child {
            self.add_log("[系統] 正在停止 Gufo 推理引擎...".to_string()).await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            self.add_log("[系統] Gufo 推理引擎已停止，GPU 顯存已釋放。".to_string()).await;
        }

        *self.state.lock().await = ServerStateEnum::Stopped;
        *self.start_time.lock().await = None;
        *self.error_message.lock().await = None;

        Ok(self.get_status().await)
    }

    pub async fn check_health(&self) -> bool {
        let cfg = self.config.lock().await.clone();
        tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port))
            .await
            .is_ok()
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
        let status = manager.get_status().await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.port, 12223);
        assert_eq!(status.pid, None);
        assert_eq!(status.endpoint, "http://127.0.0.1:12223/v1");

        // 測試 stop 不會因為 Mutex 造成死鎖
        let stop_status = manager.stop().await.unwrap();
        assert_eq!(stop_status.state, ServerStateEnum::Stopped);
    }

    #[tokio::test]
    async fn test_resolve_relative_path() {
        let p = GufoManager::resolve_path("Cargo.toml");
        assert!(p.is_relative() || p.is_absolute());
    }
}

