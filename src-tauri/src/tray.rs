use crate::gufo_manager::ServerStateEnum;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

#[allow(dead_code)]
pub struct TrayMenuState {
    pub status_item: MenuItem<Wry>,
    pub toggle_server_item: MenuItem<Wry>,
}

pub fn format_tray_labels(
    server_state: &ServerStateEnum,
    port: u16,
) -> (String, &'static str, String) {
    match server_state {
        ServerStateEnum::Running => (
            format!("狀態: 運行中 (Port {})", port),
            "停止推理伺服器",
            format!("Gufo Studio - 運行中 (Port {})", port),
        ),
        ServerStateEnum::Starting => (
            format!("狀態: 啟動中... (Port {})", port),
            "停止推理伺服器",
            format!("Gufo Studio - 啟動中... (Port {})", port),
        ),
        ServerStateEnum::Error => (
            format!("狀態: 發生錯誤 (Port {})", port),
            "啟動推理伺服器",
            format!("Gufo Studio - 發生錯誤 (Port {})", port),
        ),
        ServerStateEnum::Stopped => (
            format!("狀態: 已停止 (Port {})", port),
            "啟動推理伺服器",
            format!("Gufo Studio - 已停止 (Port {})", port),
        ),
    }
}

pub fn setup_tray(app: &AppHandle, initial_port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let title_item = MenuItem::with_id(app, "title", "🦉 Gufo Studio (Wrapper)", false, None::<&str>)?;
    let status_item = MenuItem::with_id(
        app,
        "status",
        format!("狀態: 已停止 (Port {})", initial_port),
        false,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let open_item = MenuItem::with_id(app, "open", "開啟主介面", true, None::<&str>)?;
    let toggle_server_item =
        MenuItem::with_id(app, "toggle_server", "啟動推理伺服器", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 Gufo Studio", true, None::<&str>)?;

    app.manage(TrayMenuState {
        status_item: status_item.clone(),
        toggle_server_item: toggle_server_item.clone(),
    });

    let menu = Menu::with_items(
        app,
        &[
            &title_item,
            &status_item,
            &sep1,
            &open_item,
            &toggle_server_item,
            &sep2,
            &quit_item,
        ],
    )?;

    let _tray = TrayIconBuilder::with_id("gufo-tray")
        .tooltip(format!("Gufo Studio - 已停止 (Port {})", initial_port))
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
                "toggle_server" => {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app_clone.state::<crate::AppState>();
                        let status = state.manager.get_status(Some(&app_clone)).await;
                        match status.state {
                            ServerStateEnum::Running | ServerStateEnum::Starting => {
                                let _ = state.manager.stop(Some(&app_clone)).await;
                            }
                            ServerStateEnum::Stopped | ServerStateEnum::Error => {
                                let _ = state.manager.start(app_clone.clone(), None).await;
                            }
                        }
                    });
                }
                "quit" => {
                    let app_clone = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(window) = app_clone.get_webview_window("main") {
                            crate::window_state::save_window_state(&window);
                        }
                        let state = app_clone.state::<crate::AppState>();
                        let _ = state.manager.stop(Some(&app_clone)).await;
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

pub fn update_tray_status(app: &AppHandle, server_state: &ServerStateEnum, port: u16) {
    #[cfg(test)]
    {
        let _ = (app, server_state, port);
    }

    #[cfg(not(test))]
    {
        let (status_text, toggle_text, tooltip_text) = format_tray_labels(server_state, port);

        if let Some(tray_state) = app.try_state::<TrayMenuState>() {
            let _ = tray_state.status_item.set_text(&status_text);
            let _ = tray_state.toggle_server_item.set_text(toggle_text);
        }

        if let Some(tray) = app.tray_by_id("gufo-tray") {
            let _ = tray.set_tooltip(Some(&tooltip_text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tray_labels_by_state() {
        let (status, toggle, _) = format_tray_labels(&ServerStateEnum::Stopped, 12223);
        assert_eq!(status, "狀態: 已停止 (Port 12223)");
        assert_eq!(toggle, "啟動推理伺服器");

        let (status, toggle, _) = format_tray_labels(&ServerStateEnum::Starting, 12223);
        assert_eq!(status, "狀態: 啟動中... (Port 12223)");
        assert_eq!(toggle, "停止推理伺服器");

        let (status, toggle, _) = format_tray_labels(&ServerStateEnum::Running, 12223);
        assert_eq!(status, "狀態: 運行中 (Port 12223)");
        assert_eq!(toggle, "停止推理伺服器");

        let (status, toggle, _) = format_tray_labels(&ServerStateEnum::Error, 12223);
        assert_eq!(status, "狀態: 發生錯誤 (Port 12223)");
        assert_eq!(toggle, "啟動推理伺服器");
    }
}
