use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 1200,
            height: 840,
            is_maximized: false,
        }
    }
}

/// 取得視窗狀態儲存檔案路徑 (便攜存放於 exe 同目錄或當前工作目錄)
pub fn get_window_state_file_path() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    exe_dir.join("gufo-studio.window.json")
}

/// 從磁碟載入上次儲存的視窗狀態
pub fn load_window_state() -> Option<WindowState> {
    let path = get_window_state_file_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(state) = serde_json::from_str::<WindowState>(&content) {
                return Some(state);
            }
        }
    }
    None
}

/// 儲存當前視窗狀態至磁碟
pub fn save_window_state(window: &WebviewWindow) {
    let is_maximized = window.is_maximized().unwrap_or(false);
    let mut state = load_window_state().unwrap_or_default();
    state.is_maximized = is_maximized;

    // 只有在非最小化且非最大化狀態下，才儲存常態幾何座標與尺寸
    let is_minimized = window.is_minimized().unwrap_or(false);
    if !is_minimized && !is_maximized {
        if let Ok(pos) = window.outer_position() {
            state.x = pos.x;
            state.y = pos.y;
        }
        if let Ok(size) = window.outer_size() {
            if size.width >= 400 && size.height >= 300 {
                state.width = size.width;
                state.height = size.height;
            }
        }
    }

    let path = get_window_state_file_path();
    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(path, json);
    }
}

/// 計算視窗與螢幕矩形之重疊有效性
pub fn check_rect_overlap(
    win_x: i32,
    win_y: i32,
    win_w: i32,
    win_h: i32,
    mon_x: i32,
    mon_y: i32,
    mon_w: i32,
    mon_h: i32,
) -> bool {
    let overlap_x1 = win_x.max(mon_x);
    let overlap_y1 = win_y.max(mon_y);
    let overlap_x2 = (win_x + win_w).min(mon_x + mon_w);
    let overlap_y2 = (win_y + win_h).min(mon_y + mon_h);

    let overlap_w = overlap_x2 - overlap_x1;
    let overlap_h = overlap_y2 - overlap_y1;

    // 至少有 100x60 像素落入顯示器範圍內
    if overlap_w >= 100 && overlap_h >= 60 {
        // 確保視窗頂部不會超出螢幕頂端過多，以維持標題列可操作性
        if win_y >= mon_y - 30 && win_y < mon_y + mon_h {
            return true;
        }
    }
    false
}

/// 判斷視窗是否位於任一可用的顯示器內
pub fn is_within_any_monitor(window: &WebviewWindow, win_x: i32, win_y: i32, win_w: u32, win_h: u32) -> bool {
    let monitors = match window.available_monitors() {
        Ok(m) if !m.is_empty() => m,
        _ => return false,
    };

    let win_w_i32 = win_w as i32;
    let win_h_i32 = win_h as i32;

    for mon in monitors {
        let mon_pos = mon.position();
        let mon_size = mon.size();

        if check_rect_overlap(
            win_x,
            win_y,
            win_w_i32,
            win_h_i32,
            mon_pos.x,
            mon_pos.y,
            mon_size.width as i32,
            mon_size.height as i32,
        ) {
            return true;
        }
    }

    false
}

/// 當視窗超出可見區域時，將視窗移動並置中於第一個螢幕 (Primary Monitor)
pub fn move_to_first_monitor(window: &WebviewWindow, win_w: u32, win_h: u32) {
    let first_monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| {
            window
                .available_monitors()
                .ok()
                .and_then(|m| m.into_iter().next())
        });

    if let Some(mon) = first_monitor {
        let mon_pos = mon.position();
        let mon_size = mon.size();

        // 確保寬高不超過螢幕工作區域
        let safe_w = win_w.min(mon_size.width.saturating_sub(60)).max(800);
        let safe_h = win_h.min(mon_size.height.saturating_sub(80)).max(600);

        let _ = window.set_size(Size::Physical(PhysicalSize {
            width: safe_w,
            height: safe_h,
        }));

        // 置中放置在第一個螢幕
        let center_x = mon_pos.x + ((mon_size.width as i32 - safe_w as i32) / 2).max(0);
        let center_y = mon_pos.y + ((mon_size.height as i32 - safe_h as i32) / 2).max(0);

        let _ = window.set_position(Position::Physical(PhysicalPosition {
            x: center_x,
            y: center_y,
        }));
    } else {
        let _ = window.center();
    }
}

/// 啟動時還原視窗狀態：
/// 1. 若在現有螢幕範圍內，還原歷史位置與尺寸；
/// 2. 若超出顯示區域（如外接螢幕拔除），則安全移動至第一個螢幕並置中；
/// 3. 若歷史狀態為最大化，則恢復最大化。
pub fn restore_window_state(window: &WebviewWindow) {
    let saved = match load_window_state() {
        Some(s) => s,
        None => return, // 若無歷史紀錄，保留 tauri.conf.json 預設值
    };

    let in_bounds = is_within_any_monitor(window, saved.x, saved.y, saved.width, saved.height);

    if in_bounds {
        let _ = window.set_size(Size::Physical(PhysicalSize {
            width: saved.width,
            height: saved.height,
        }));
        let _ = window.set_position(Position::Physical(PhysicalPosition {
            x: saved.x,
            y: saved.y,
        }));
    } else {
        // 超出顯示區域，降級至第一個螢幕
        move_to_first_monitor(window, saved.width, saved.height);
    }

    if saved.is_maximized {
        let _ = window.maximize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_state_default() {
        let state = WindowState::default();
        assert_eq!(state.width, 1200);
        assert_eq!(state.height, 840);
        assert!(!state.is_maximized);
    }

    #[test]
    fn test_check_rect_overlap_inside() {
        // 視窗完全在 1920x1080 螢幕內
        let is_ok = check_rect_overlap(100, 100, 1200, 800, 0, 0, 1920, 1080);
        assert!(is_ok);
    }

    #[test]
    fn test_check_rect_overlap_outside_disconnected_monitor() {
        // 視窗原本在第二螢幕 (x: 1920~3840)，但現在只剩主螢幕 (0~1920)
        let is_ok = check_rect_overlap(2100, 100, 1200, 800, 0, 0, 1920, 1080);
        assert!(!is_ok);
    }

    #[test]
    fn test_check_rect_overlap_off_screen_top() {
        // 視窗跑到太上方導致無法點擊標題列
        let is_ok = check_rect_overlap(100, -200, 1200, 800, 0, 0, 1920, 1080);
        assert!(!is_ok);
    }
}
