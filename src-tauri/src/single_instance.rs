//! 单实例保护（Windows）。
//!
//! 以前可以同时启动两个桌宠进程：两套定时任务调度器（同一条提醒触发两次）、两只桌宠、
//! 两套全局快捷键，还会同时写同一个 pet.db。这里用具名互斥量保证同一用户只有一个实例；
//! 第二次启动时把已运行实例的控制台窗口调到前台，然后直接退出。
//! debug / release 用不同的名字（和 WebView 数据目录的区分一致），开发版和安装版互不阻挡。

#[cfg(windows)]
pub fn ensure_single_instance() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = if cfg!(debug_assertions) {
        "Local\\ai-desktop-pet-dev-singleton"
    } else {
        "Local\\ai-desktop-pet-singleton"
    };
    let wide = to_wide(name);
    // SAFETY: 名字是以 0 结尾的 UTF-16；不传安全属性。
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
        if handle.is_null() {
            // 创建失败（极少见）时不阻止启动。
            return true;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            focus_existing_instance();
            return false;
        }
        // 句柄故意不关闭：进程存活期间一直持有互斥量，退出时由系统回收。
    }
    true
}

#[cfg(not(windows))]
pub fn ensure_single_instance() -> bool {
    true
}

#[cfg(windows)]
fn focus_existing_instance() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, MessageBoxW, SetForegroundWindow, ShowWindow, MB_ICONINFORMATION,
        MB_OK, SW_RESTORE, SW_SHOW,
    };

    // 与 tauri.conf.json 里主窗口（控制台）的标题一致。
    let title = to_wide("AI Desktop Pet - 控制台");
    // SAFETY: 字符串都以 0 结尾；窗口句柄只用于显示与置前。
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            let text = to_wide("AI Desktop Pet 已经在运行，可以在桌宠或系统托盘里打开控制台。");
            let caption = to_wide("AI Desktop Pet");
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONINFORMATION);
            return;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else {
            ShowWindow(hwnd, SW_SHOW);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(windows)]
fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
