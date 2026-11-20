mod commands;
pub mod gufo_manager;
pub mod https_proxy;
mod process_control;
pub mod rag;
mod tray;
pub mod window_state;

use gufo_manager::GufoManager;
use rag::RagManager;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{Manager, WindowEvent};

/// 視窗移動/縮放事件會連續大量觸發，延遲 500ms 後只寫入最後一次狀態。
const WINDOW_STATE_SAVE_DEBOUNCE_MS: u64 = 500;

pub struct AppState {
    pub manager: Arc<GufoManager>,
    pub rag: Arc<RagManager>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let manager = Arc::new(GufoManager::new());
    let rag = Arc::new(RagManager::new());
    let manager_for_exit = manager.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            manager: manager.clone(),
            rag,
        })
        .setup(|app| {
            let initial_config = gufo_manager::GufoManager::load_saved_config().unwrap_or_default();

            // 初始化系統匣 (Tray Icon)
            tray::setup_tray(
                app.handle(),
                initial_config.port,
                initial_config.https_enabled.then_some(initial_config.https_port),
            )?;

            // 監聽視窗關閉事件：關閉時隱藏視窗至 Tray，而非直接退出
            if let Some(window) = app.get_webview_window("main") {
                // 啟動時還原上次關閉時的位置與尺寸，若超出螢幕則移動至第一個螢幕
                window_state::restore_window_state(&window);

                // 若設定不自動開啟 GUI，則啟動後自動隱藏至系統匣 (Tray)
                if !initial_config.auto_open_gui {
                    let _ = window.hide();
                }

                let win_clone = window.clone();
                let save_ticket = Arc::new(AtomicU64::new(0));
                window.on_window_event(move |event| {
                    match event {
                        WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            // 隱藏至 Tray 前記錄當前視窗狀態
                            save_ticket.fetch_add(1, Ordering::SeqCst);
                            window_state::save_window_state(&win_clone);
                            let _ = win_clone.hide();
                        }
                        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                            let ticket = save_ticket.fetch_add(1, Ordering::SeqCst) + 1;
                            let ticket_ref = save_ticket.clone();
                            let win = win_clone.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    WINDOW_STATE_SAVE_DEBOUNCE_MS,
                                ))
                                .await;
                                if ticket_ref.load(Ordering::SeqCst) == ticket {
                                    window_state::save_window_state(&win);
                                }
                            });
                        }
                        _ => {}
                    }
                });
            }

            // 檢查是否啟動後自動啟動推論伺服器，否則主動探測背景是否已有運行中的 Gufo 伺服器以同步狀態
            let app_handle = app.handle().clone();
            let state = app.state::<AppState>();
            let manager_clone = state.manager.clone();
            let auto_start = initial_config.auto_start_server;
            tauri::async_runtime::spawn(async move {
                // 稍微延遲 500ms 確保系統匣、前端與事件通道就緒
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if auto_start {
                    let _ = manager_clone.start(app_handle, None).await;
                } else {
                    manager_clone.sync_existing_server_on_startup(app_handle).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_default_config,
            commands::get_config,
            commands::has_saved_config,
            commands::save_config,
            commands::get_server_status,
            commands::start_server,
            commands::stop_server,
            commands::check_health,
            commands::regenerate_https_cert,
            commands::get_recent_logs,
            commands::get_recent_metrics,
            commands::check_path_exists,
            commands::select_file,
            commands::rag_get_config,
            commands::rag_save_config,
            commands::rag_test_embedding,
            commands::rag_list_kbs,
            commands::rag_get_kb,
            commands::rag_create_kb,
            commands::rag_delete_kb,
            commands::rag_pick_files,
            commands::rag_pick_folder,
            commands::rag_add_files,
            commands::rag_add_folder,
            commands::rag_add_text,
            commands::rag_get_doc_chunks,
            commands::rag_delete_document,
            commands::rag_reindex_kb,
            commands::rag_search,
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
                    let _ = manager.stop(None).await;
                });
            }
        });
}
