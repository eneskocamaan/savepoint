//! Process-level Win32 helpers: single instance, priorities, autostart, and
//! finding or launching the UI.

use savepoint_core::win::{HKEY_CURRENT_USER, reg_delete_value, reg_get_string, reg_set_string};
use std::ffi::c_void;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetCurrentThread, OpenProcess, PROCESS_INFORMATION_CLASS,
    PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_NAME_WIN32, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION,
    ProcessPowerThrottling, QueryFullProcessImageNameW, SetPriorityClass, SetProcessInformation, SetThreadPriority,
    THREAD_MODE_BACKGROUND_BEGIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
    SW_RESTORE, SetForegroundWindow, ShowWindow, WM_CLOSE,
};
use windows::core::{BOOL, PWSTR, w};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Savepoint";
const UI_EXE: &str = "savepoint-ui.exe";

/// False if another savepointd is already running for this user.
pub fn acquire_single_instance() -> bool {
    unsafe {
        let mutex = CreateMutexW(None, true, w!("Local\\SavepointDaemon"));
        mutex.is_ok() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Lets the process that receives our next request bring its window forward.
pub fn allow_foreground() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// Asks Windows to run the whole service at low clock speed (EcoQoS).
pub fn efficiency_mode() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    set_process_info(ProcessPowerThrottling, &state);
}

fn set_process_info<T>(class: PROCESS_INFORMATION_CLASS, value: &T) {
    unsafe {
        let _ = SetProcessInformation(
            GetCurrentProcess(),
            class,
            value as *const T as *const c_void,
            size_of::<T>() as u32,
        );
    }
}

/// Low CPU and I/O priority for the whole process (used by `--scan`).
pub fn background_process() {
    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
    }
}

/// Low CPU and I/O priority for the calling thread, so backups yield to games.
pub fn background_thread() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

pub fn autostart_enabled() -> bool {
    reg_get_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE).is_some()
}

pub fn set_autostart(enabled: bool) -> io::Result<()> {
    if enabled {
        let exe = std::env::current_exe()?;
        let command = format!("\"{}\"", exe.display());
        if !reg_set_string(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, &command) {
            return Err(io::Error::other("autostart_failed"));
        }
    } else {
        reg_delete_value(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
    }
    Ok(())
}

fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Into::into)).unwrap_or_default()
}

/// Focuses the open UI window, or starts the UI if none is open.
pub fn open_ui() {
    let windows = ui_windows();
    if let Some(&hwnd) = windows.first() {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    // We were just clicked, so we may hand foreground rights to the flyout.
    allow_foreground();
    if let Err(e) = Command::new(exe_dir().join(UI_EXE)).spawn() {
        crate::log::log!("could not start UI: {e}");
    }
}

pub fn close_ui() {
    for hwnd in ui_windows() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Visible top-level windows that belong to a `savepoint-ui.exe` process.
fn ui_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if process_is_ui(pid) {
                found.push(hwnd);
            }
        }
        BOOL(1)
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut found as *mut Vec<HWND> as isize));
    }
    found
}

fn process_is_ui(pid: u32) -> bool {
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    ok.is_ok() && String::from_utf16_lossy(&buf[..len as usize]).to_lowercase().ends_with(UI_EXE)
}
