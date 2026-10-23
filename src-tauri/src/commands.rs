use crate::gufo_manager::{GufoServerConfig, ServerStatus};
use crate::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn get_default_config(state: State<'_, AppState>) -> Result<GufoServerConfig, String> {
    Ok(state.manager.get_config().await)
}

#[tauri::command]
pub async fn save_config(
    state: State<'_, AppState>,
    config: GufoServerConfig,
) -> Result<(), String> {
    state.manager.save_config(config).await
}

#[tauri::command]
pub async fn get_server_status(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    Ok(state.manager.get_status().await)
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
pub async fn stop_server(state: State<'_, AppState>) -> Result<ServerStatus, String> {
    state.manager.stop().await
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
