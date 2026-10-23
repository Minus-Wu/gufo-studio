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
}

impl Default for GufoServerConfig {
    fn default() -> Self {
        // 預設檢查常用的 Gufo 本機路徑
        let default_gufo = if std::path::Path::new(r"D:\Agents\gufo\build\release\gufo.exe").exists() {
            r"D:\Agents\gufo\build\release\gufo.exe".to_string()
        } else if std::path::Path::new(r"C:\gufo\gufo.exe").exists() {
            r"C:\gufo\gufo.exe".to_string()
        } else if std::path::Path::new("gufo.exe").exists() {
            "gufo.exe".to_string()
        } else {
            r"D:\Agents\gufo\build\release\gufo.exe".to_string()
        };

        let default_model_dir = PathBuf::from(r"D:\Models\GGUF\Qwen3.8-Flash-Next-GGUF");
        let model_file = default_model_dir.join("Qwen3.8-Flash-Next-UD-Q4_K_XL-00001-of-00004.gguf");
        let mtp_file = default_model_dir.join("mtp-Qwen3.8-Flash-Next-shared-Q8_0.gguf");
        let vision_file = default_model_dir.join("mmproj-BF16.gguf");

        Self {
            gufo_path: default_gufo,
            model_path: model_file.to_string_lossy().to_string(),
            mtp_path: mtp_file.to_string_lossy().to_string(),
            vision_path: vision_file.to_string_lossy().to_string(),
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
        Self {
            child: Arc::new(Mutex::new(None)),
            config: Arc::new(Mutex::new(GufoServerConfig::default())),
            state: Arc::new(Mutex::new(ServerStateEnum::Stopped)),
            start_time: Arc::new(Mutex::new(None)),
            error_message: Arc::new(Mutex::new(None)),
            recent_logs: Arc::new(Mutex::new(VecDeque::with_capacity(1000))),
        }
    }

    pub async fn get_config(&self) -> GufoServerConfig {
        self.config.lock().await.clone()
    }

    pub async fn get_status(&self) -> ServerStatus {
        let state = self.state.lock().await.clone();
        let cfg = self.config.lock().await.clone();
        let child_guard = self.child.lock().await;
        let pid = child_guard.as_ref().and_then(|c| c.id());
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
        }
    }

    pub async fn get_logs(&self) -> Vec<String> {
        self.recent_logs.lock().await.iter().cloned().collect()
    }

    pub async fn start(&self, app: AppHandle, new_config: Option<GufoServerConfig>) -> Result<ServerStatus, String> {
        let mut child_guard = self.child.lock().await;
        if child_guard.is_some() {
            return Err("伺服器已在運行中".to_string());
        }

        if let Some(cfg) = new_config {
            *self.config.lock().await = cfg;
        }
        let cfg = self.config.lock().await.clone();

        // 檢查執行檔是否存在
        if !std::path::Path::new(&cfg.gufo_path).exists() {
            let msg = format!("找不到 Gufo 主程式檔案：{}", cfg.gufo_path);
            *self.state.lock().await = ServerStateEnum::Error;
            *self.error_message.lock().await = Some(msg.clone());
            return Err(msg);
        }

        // 檢查模型檔案是否存在
        if !std::path::Path::new(&cfg.model_path).exists() {
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
            cfg.model_path.clone(),
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

        if cfg.speculative == "mtp" && !cfg.mtp_path.is_empty() && std::path::Path::new(&cfg.mtp_path).exists() {
            args.push("--speculative".to_string());
            args.push("mtp".to_string());
            args.push("--mtp-model".to_string());
            args.push(cfg.mtp_path.clone());
            if !cfg.mtp_policy.is_empty() {
                args.push("--mtp-policy".to_string());
                args.push(cfg.mtp_policy.clone());
            }
        }

        if cfg.prompt_lookup {
            args.push("--prompt-lookup".to_string());
        }

        if !cfg.vision_path.is_empty() && std::path::Path::new(&cfg.vision_path).exists() {
            args.push("--mmproj".to_string());
            args.push(cfg.vision_path.clone());
        }

        let cmd_display = format!("{} {}", cfg.gufo_path, args.join(" "));
        self.add_log(format!("[系統] 準備啟動：{}", cmd_display)).await;

        let mut command = Command::new(&cfg.gufo_path);
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

        *child_guard = Some(spawned);
        *self.start_time.lock().await = Some(Instant::now());

        // 啟動 stdout 讀取任務
        let logs_clone1 = self.recent_logs.clone();
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
                    let _ = app_clone1.emit("gufo-log", line);
                }
            });
        }

        // 啟動 stderr 讀取任務
        let logs_clone2 = self.recent_logs.clone();
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
                    let _ = app_clone2.emit("gufo-log", line);
                }
            });
        }

        // 啟動健康檢查背景監控
        let state_clone = self.state.clone();
        let host = cfg.host.clone();
        let port = cfg.port;
        let app_clone3 = app.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1500))
                .build()
                .unwrap_or_default();
            let url = format!("http://{}:{}/v1/models", host, port);

            // 每 1 秒檢查一次，最多嘗試 60 次
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                if let Ok(resp) = client.get(&url).send().await {
                    if resp.status().is_success() {
                        *state_clone.lock().await = ServerStateEnum::Running;
                        let _ = app_clone3.emit("gufo-status-changed", "running");
                        return;
                    }
                }
            }
            // 若 60 秒都沒回應且仍為 Starting，標記為 Running (因為有些版本只在完成推論時回覆)
            let mut st = state_clone.lock().await;
            if *st == ServerStateEnum::Starting {
                *st = ServerStateEnum::Running;
                let _ = app_clone3.emit("gufo-status-changed", "running");
            }
        });

        Ok(self.get_status().await)
    }

    pub async fn stop(&self) -> Result<ServerStatus, String> {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
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
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1500))
            .build()
            .unwrap_or_default();
        let url = format!("http://{}:{}/v1/models", cfg.host, cfg.port);
        match client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
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
    }

    #[tokio::test]
    async fn test_manager_initial_status() {
        let manager = GufoManager::new();
        let status = manager.get_status().await;
        assert_eq!(status.state, ServerStateEnum::Stopped);
        assert_eq!(status.port, 12223);
        assert_eq!(status.pid, None);
        assert_eq!(status.endpoint, "http://127.0.0.1:12223/v1");
    }
}

