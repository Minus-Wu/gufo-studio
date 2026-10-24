use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let title_item = MenuItem::with_id(app, "title", "🦉 Gufo Studio (Wrapper)", false, None::<&str>)?;
    let status_item = MenuItem::with_id(app, "status", "狀態: 已停止 (Port 12223)", false, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let open_item = MenuItem::with_id(app, "open", "開啟主介面", true, None::<&str>)?;
    let start_server_item = MenuItem::with_id(app, "start_server", "啟動推理伺服器", true, None::<&str>)?;
    let stop_server_item = MenuItem::with_id(app, "stop_server", "停止推理伺服器", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 Gufo Studio", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &title_item,
            &status_item,
            &sep1,
            &open_item,
            &start_server_item,
            &stop_server_item,
            &sep2,
            &quit_item,
        ],
    )?;

    let _tray = TrayIconBuilder::with_id("gufo-tray")
        .tooltip("Gufo Studio - AMD Strix Halo 推理控制中心")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            match event.id.as_ref() {
                "open" => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                }
                "start_server" => {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_clone.state::<crate::AppState>();
                        let _ = state.manager.start(app_clone.clone(), None).await;
                    });
                }
                "stop_server" => {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_clone.state::<crate::AppState>();
                        let _ = state.manager.stop().await;
                    });
                }
                "quit" => {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(window) = app_clone.get_webview_window("main") {
                            crate::window_state::save_window_state(&window);
                        }
                        let state = app_clone.state::<crate::AppState>();
                        let _ = state.manager.stop().await;
                        app_clone.exit(0);
                    });
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if let Ok(visible) = window.is_visible() {
                        if visible {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}
