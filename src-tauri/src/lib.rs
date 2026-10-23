mod commands;
pub mod gufo_manager;
mod tray;

use gufo_manager::GufoManager;
use std::sync::Arc;
use tauri::{Manager, WindowEvent};

pub struct AppState {
    pub manager: Arc<GufoManager>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let manager = Arc::new(GufoManager::new());
    let manager_for_exit = manager.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            manager: manager.clone(),
        })
        .setup(|app| {
            // 初始化系統匣 (Tray Icon)
            tray::setup_tray(app.handle())?;

            // 監聽視窗關閉事件：關閉時隱藏視窗至 Tray，而非直接退出
            if let Some(window) = app.get_webview_window("main") {
                let win_clone = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_default_config,
            commands::save_config,
            commands::get_server_status,
            commands::start_server,
            commands::stop_server,
            commands::check_health,
            commands::get_recent_logs,
            commands::check_path_exists,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // 當 App 徹底退出時，確保終止 gufo.exe 子程序，避免 GPU 顯存殘留
                let manager = manager_for_exit.clone();
                tauri::async_runtime::block_on(async move {
                    let _ = manager.stop().await;
                });
            }
        });
}
