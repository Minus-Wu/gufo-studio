use crate::gufo_manager::{GufoServerConfig, InferenceMetrics, ServerStatus};
use crate::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn get_default_config(state: State<'_, AppState>) -> Result<GufoServerConfig, String> {
    Ok(state.manager.get_config().await)
}

#[tauri::command]
pub async fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: GufoServerConfig,
) -> Result<(), String> {
    state.manager.save_config(Some(&app), config).await
}

#[tauri::command]
pub async fn get_server_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ServerStatus, String> {
    Ok(state.manager.get_status(Some(&app)).await)
}

#[tauri::command]
pub async fn start_server(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Option<GufoServerConfig>,
) -> Result<ServerStatus, String> {
    state.manager.start(app, config).await
}

#[tauri::command]
pub async fn stop_server(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ServerStatus, String> {
    state.manager.stop(Some(&app)).await
}

#[tauri::command]
pub async fn check_health(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.manager.check_health().await)
}

#[tauri::command]
pub async fn get_recent_logs(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.manager.get_logs().await)
}

#[tauri::command]
pub async fn get_recent_metrics(state: State<'_, AppState>) -> Result<Vec<InferenceMetrics>, String> {
    Ok(state.manager.get_metrics().await)
}

#[tauri::command]
pub fn check_path_exists(path: String) -> bool {
    let p = std::path::Path::new(&path);
    if p.exists() {
        return true;
    }
    if p.is_relative() {
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join(p).exists() {
                return true;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                if parent.join(p).exists() {
                    return true;
                }
            }
        }
    }
    false
}

#[tauri::command]
pub async fn select_file(
    title: Option<String>,
    filter_name: Option<String>,
    extensions: Option<Vec<String>>,
    default_path: Option<String>,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new();
        if let Some(t) = title {
            dialog = dialog.set_title(&t);
        }
        if let (Some(name), Some(exts)) = (filter_name, extensions) {
            let str_slices: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
            dialog = dialog.add_filter(&name, &str_slices);
        }
        if let Some(def) = default_path {
            let p = std::path::Path::new(&def);
            if p.exists() {
                if p.is_dir() {
                    dialog = dialog.set_directory(p);
                } else if let Some(parent) = p.parent() {
                    dialog = dialog.set_directory(parent);
                }
            } else if let Ok(cwd) = std::env::current_dir() {
                let full = cwd.join(p);
                if let Some(parent) = full.parent() {
                    if parent.exists() {
                        dialog = dialog.set_directory(parent);
                    }
                }
            }
        } else if let Ok(cwd) = std::env::current_dir() {
            dialog = dialog.set_directory(&cwd);
        }

        let picked = dialog.pick_file();
        if let Some(path_buf) = picked {
            // 若選取檔案位於工作目錄下，優先轉為簡潔的相對路徑
            if let Ok(cwd) = std::env::current_dir() {
                if let Ok(rel) = path_buf.strip_prefix(&cwd) {
                    return Ok(Some(rel.to_string_lossy().to_string()));
                }
            }
            return Ok(Some(path_buf.to_string_lossy().to_string()));
        }
        Ok(None)
    })
    .await
    .map_err(|e| e.to_string())?
}
