//! 針對單一 PID 的程序控制工具。
//!
//! 只會操作「指定 PID」或「在指定 Port 上監聽且映像名稱為 Gufo」的程序，
//! 絕不以映像名稱 (例如 `taskkill /IM gufo.exe`) 進行大範圍終止，
//! 避免誤殺使用者在其他終端或其他 Port 上手動啟動的 Gufo 實例。
//! 所有函式皆為同步阻塞呼叫，請於 `spawn_blocking` 中使用。

use std::path::Path;
use std::time::Duration;

pub use imp::{find_listening_pid, force_kill, process_image_name, request_graceful_stop, wait_for_exit};

/// 從 `netstat -ano` 輸出中找出在指定 Port 上監聽的 TCP 程序 PID。
/// 狀態欄位會依系統語系翻譯 (例如「LISTENING」/「接聽」)，因此改以
/// 遠端位址為 `0.0.0.0:0` / `[::]:0` 判斷監聽中的 socket。
pub fn parse_netstat_listening_pid(output: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{}", port);
    output.lines().find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 || !cols[0].eq_ignore_ascii_case("TCP") {
            return None;
        }
        if !cols[1].ends_with(&suffix) || !matches!(cols[2], "0.0.0.0:0" | "[::]:0") {
            return None;
        }
        cols.last()?.parse::<u32>().ok().filter(|pid| *pid != 0)
    })
}

/// 判斷映像名稱是否屬於預期的 Gufo 執行檔 (不分大小寫)。
pub fn is_expected_image(image_name: &str, expected: &[String]) -> bool {
    let name = image_name.trim();
    !name.is_empty() && expected.iter().any(|e| e.eq_ignore_ascii_case(name))
}

/// 由設定的 gufo 路徑推導出可接受的程序映像名稱。
pub fn expected_gufo_images(configured_path: &str) -> Vec<String> {
    let mut names = vec!["gufo.exe".to_string(), "gufo".to_string()];
    if let Some(file) = Path::new(configured_path).file_name() {
        let file = file.to_string_lossy().to_string();
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&file)) {
            names.push(file);
        }
    }
    names
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
        GetConsoleWindow, SetConsoleCtrlHandler,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// 主控台附著屬於整個程序的全域狀態，必須序列化。
    static CONSOLE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct ProcessHandle(HANDLE);

    impl ProcessHandle {
        fn open(pid: u32, access: u32) -> Option<Self> {
            let handle = unsafe { OpenProcess(access, 0, pid) };
            (!handle.is_null()).then_some(Self(handle))
        }
    }

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn find_listening_pid(port: u16) -> Option<u32> {
        let output = std::process::Command::new("netstat")
            .args(["-ano"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        super::parse_netstat_listening_pid(&String::from_utf8_lossy(&output.stdout), port)
    }

    pub fn process_image_name(pid: u32) -> Option<String> {
        let handle = ProcessHandle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        if ok == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
    }

    /// 對目標程序的主控台送出 Ctrl+C，讓 Gufo 走 SIGINT 正常關閉流程
    /// (排空磁碟快取寫入並釋放 GPU 記憶體)。
    /// 子程序以 CREATE_NO_WINDOW 啟動，擁有自己的隱藏主控台，因此只會影響 Gufo 本身。
    pub fn request_graceful_stop(pid: u32) -> bool {
        let _guard = CONSOLE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let had_console = !GetConsoleWindow().is_null();
            FreeConsole();
            if AttachConsole(pid) == 0 {
                if had_console {
                    AttachConsole(ATTACH_PARENT_PROCESS);
                }
                return false;
            }
            // 暫時忽略 Ctrl+C，避免 Gufo Studio 自己也收到事件而結束
            SetConsoleCtrlHandler(None, 1);
            let sent = GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0;
            // 事件為非同步派送，稍候再脫離主控台並恢復預設處理
            std::thread::sleep(Duration::from_millis(300));
            FreeConsole();
            SetConsoleCtrlHandler(None, 0);
            if had_console {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
            sent
        }
    }

    pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
        let Some(handle) = ProcessHandle::open(pid, PROCESS_SYNCHRONIZE) else {
            return true;
        };
        let ms = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        unsafe { WaitForSingleObject(handle.0, ms) == WAIT_OBJECT_0 }
    }

    pub fn force_kill(pid: u32) -> bool {
        match ProcessHandle::open(pid, PROCESS_TERMINATE) {
            Some(handle) => unsafe { TerminateProcess(handle.0, 1) != 0 },
            None => false,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::Command;
    use std::time::{Duration, Instant};

    pub fn find_listening_pid(port: u16) -> Option<u32> {
        let output = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{}", port), "-sTCP:LISTEN", "-t"])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|l| l.trim().parse::<u32>().ok())
    }

    pub fn process_image_name(pid: u32) -> Option<String> {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .ok()?;
        let comm = String::from_utf8_lossy(&output.stdout).trim().to_string();
        std::path::Path::new(&comm)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
    }

    fn signal(pid: u32, sig: &str) -> bool {
        Command::new("kill")
            .args([sig, &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn request_graceful_stop(pid: u32) -> bool {
        signal(pid, "-INT")
    }

    pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !signal(pid, "-0") {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        !signal(pid, "-0")
    }

    pub fn force_kill(pid: u32) -> bool {
        signal(pid, "-KILL")
    }
}

/// 停止在指定 Port 監聽的外部 (已接管) Gufo 程序。
/// 先確認映像名稱為 Gufo，再嘗試正常關閉，逾時後才強制終止。
pub fn stop_gufo_on_port(port: u16, expected: &[String], graceful_timeout: Duration) -> Result<String, String> {
    let Some(pid) = find_listening_pid(port) else {
        return Ok(format!("Port {} 已無監聽程序，外部 Gufo 伺服器可能已自行結束。", port));
    };
    let image = process_image_name(pid).unwrap_or_default();
    if !is_expected_image(&image, expected) {
        return Err(format!(
            "Port {} 由 PID {} ({}) 佔用，並非 Gufo 程序，為安全起見不予終止。",
            port,
            pid,
            if image.is_empty() { "未知程序" } else { &image }
        ));
    }
    if request_graceful_stop(pid) && wait_for_exit(pid, graceful_timeout) {
        return Ok(format!("外部 Gufo 伺服器 (PID {}) 已正常關閉。", pid));
    }
    if force_kill(pid) {
        wait_for_exit(pid, Duration::from_secs(5));
        Ok(format!("外部 Gufo 伺服器 (PID {}) 未在時限內結束，已強制終止。", pid))
    } else {
        Err(format!("無法終止外部 Gufo 伺服器 (PID {})，可能權限不足。", pid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netstat_parser_finds_listener_regardless_of_locale() {
        let sample = "\r\n使用中連線\r\n\r\n  協定   本機位址               外部位址               狀態           PID\r\n  TCP    0.0.0.0:135            0.0.0.0:0              接聽           1234\r\n  TCP    127.0.0.1:12223        0.0.0.0:0              接聽           4321\r\n  TCP    127.0.0.1:12223        127.0.0.1:50000        ESTABLISHED    4321\r\n  TCP    127.0.0.1:50000        127.0.0.1:12223        ESTABLISHED    999\r\n";
        assert_eq!(parse_netstat_listening_pid(sample, 12223), Some(4321));
        assert_eq!(parse_netstat_listening_pid(sample, 135), Some(1234));
        assert_eq!(parse_netstat_listening_pid(sample, 50000), None);
        assert_eq!(parse_netstat_listening_pid(sample, 1222), None);
    }

    #[test]
    fn netstat_parser_handles_ipv6() {
        let sample = "  TCP    [::]:12223             [::]:0                 LISTENING       777\n";
        assert_eq!(parse_netstat_listening_pid(sample, 12223), Some(777));
    }

    #[test]
    fn find_listening_pid_returns_own_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert_eq!(find_listening_pid(port), Some(std::process::id()));
    }

    /// 以與 Gufo 相同的方式 (CREATE_NO_WINDOW) 啟動會回應 Ctrl+C 的程序，
    /// 確認能正常關閉它，且測試程序本身不會收到 Ctrl+C。
    #[cfg(windows)]
    #[test]
    fn graceful_stop_delivers_ctrl_c_to_hidden_console_child() {
        use std::os::windows::process::CommandExt;
        let mut child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("spawn ping");
        std::thread::sleep(Duration::from_millis(500));
        let pid = child.id();
        let image = process_image_name(pid).unwrap_or_default();
        assert!(image.eq_ignore_ascii_case("ping.exe"), "unexpected image {}", image);
        assert!(request_graceful_stop(pid), "Ctrl+C should be sent");
        let exited = wait_for_exit(pid, Duration::from_secs(5));
        if !exited {
            let _ = child.kill();
        }
        let _ = child.wait();
        assert!(exited, "child should exit after Ctrl+C");
    }

    #[test]
    fn expected_image_matching() {
        let expected = expected_gufo_images(r"build\release\gufo-custom.exe");
        assert!(is_expected_image("GUFO.EXE", &expected));
        assert!(is_expected_image("gufo-custom.exe", &expected));
        assert!(!is_expected_image("python.exe", &expected));
        assert!(!is_expected_image("", &expected));
    }
}
