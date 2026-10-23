mod commands;
pub mod gufo_manager;
mod tray;
pub mod window_state;

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
                // 啟動時還原上次關閉時的位置與尺寸，若超出螢幕則移動至第一個螢幕
                window_state::restore_window_state(&window);

                let win_clone = window.clone();
                window.on_window_event(move |event| {
                    match event {
                        WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            // 隱藏至 Tray 前記錄當前視窗狀態
                            window_state::save_window_state(&win_clone);
                            let _ = win_clone.hide();
                        }
                        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                            // 移動或縮放時持續記錄
                            window_state::save_window_state(&win_clone);
                        }
                        _ => {}
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
            commands::select_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                // 退出前確保儲存最新視窗狀態
                if let Some(window) = app_handle.get_webview_window("main") {
                    window_state::save_window_state(&window);
                }
                // 當 App 徹底退出時，確保終止 gufo.exe 子程序，避免 GPU 顯存殘留
                let manager = manager_for_exit.clone();
                tauri::async_runtime::block_on(async move {
                    let _ = manager.stop().await;
                });
            }
        });
}
