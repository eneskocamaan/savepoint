#![windows_subsystem = "windows"]

//! The Savepoint panel. It opens above the taskbar in the bottom-right corner
//! and stays until it is closed (the X button, Esc, or the tray menu).
//!
//! It holds no state of its own: every action is a request to `savepointd`,
//! which it starts if it is not running. Closing ends this process entirely,
//! so nothing stays in memory.

use savepoint_ipc::{self as ipc, Reply};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tauri::{Manager, PhysicalPosition, WebviewWindow, WindowEvent};

const DAEMON_EXE: &str = "savepointd.exe";
/// Gap between the flyout and the screen edge / taskbar, in logical pixels.
const MARGIN: f64 = 12.0;

#[tauri::command]
async fn ipc(req: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || call(&req.to_string())).await.map_err(|e| e.to_string())?
}

fn call(line: &str) -> Result<Value, String> {
    let raw = match ipc::request_raw(line) {
        Ok(raw) => raw,
        Err(e) if ipc::is_not_running(&e) => {
            start_daemon()?;
            ipc::request_raw(line).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    };
    let reply: Reply = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    match reply.ok {
        true => Ok(reply.data),
        false => Err(reply.error.unwrap_or_default()),
    }
}

fn start_daemon() -> Result<(), String> {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)).unwrap_or_default();
    Command::new(dir.join(DAEMON_EXE)).arg("--from-ui").spawn().map_err(|_| "service_start_failed".to_string())?;
    if ipc::wait_until_ready(Duration::from_secs(5)) { Ok(()) } else { Err("service_not_responding".into()) }
}

#[tauri::command]
fn pick_folder(window: tauri::Window, title: String) -> Option<String> {
    rfd::FileDialog::new().set_parent(&window).set_title(title).pick_folder().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn pick_exe(window: tauri::Window, title: String) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title(title)
        .add_filter("exe", &["exe"])
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn close_window(window: tauri::Window) {
    let _ = window.close();
}

/// Asks Windows 11 to round the window's corners and to draw its border in the
/// panel's own color. Windows clips the whole window, so no corner is ever left
/// showing a background. (Older Windows keeps square corners.)
fn round_corners(window: &WebviewWindow) {
    use std::ffi::c_void;
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    use windows::Win32::Graphics::Gdi::CreateSolidBrush;
    use windows::Win32::UI::WindowsAndMessaging::{GCLP_HBRBACKGROUND, SetClassLongPtrW};
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = HWND(hwnd.0);
    let corners = DWMWCP_ROUND;
    let border: u32 = 0x0036_312f; // #2f3136 as 0x00BBGGRR
    unsafe {
        // Windows paints the window itself white wherever the web view does not
        // reach (the thin strip at the edges); make that the panel's own color.
        let brush = CreateSolidBrush(COLORREF(0x0016_1413)); // #131416 as 0x00BBGGRR
        SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, brush.0 as isize);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corners as *const _ as *const c_void,
            size_of_val(&corners) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const _ as *const c_void,
            size_of_val(&border) as u32,
        );
    }
}

/// Puts the window in the bottom-right corner of the primary screen's work
/// area (the part not covered by the taskbar), then shows it.
fn place(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = match window.primary_monitor()? {
        Some(m) => Some(m),
        None => window.current_monitor()?,
    };
    if let Some(monitor) = monitor {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let margin = (MARGIN * monitor.scale_factor()).round() as i32;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    window.show()?;
    window.set_focus()
}

/// True if another panel is already open for this user. Two quick tray clicks
/// can start two panels before the first one has shown its window.
fn another_panel_is_open() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;
    // The handle is kept for the life of the process on purpose.
    unsafe { CreateMutexW(None, true, w!("Local\\SavepointPanel")).is_err() || GetLastError() == ERROR_ALREADY_EXISTS }
}

fn main() {
    if another_panel_is_open() {
        return;
    }
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc, pick_folder, pick_exe, close_window])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                round_corners(&window);
                place(&window)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // The flyout is the only window: when it goes, the process goes
            // too, however it was closed (X, Esc, the tray menu, or the service).
            if let WindowEvent::Destroyed = event {
                window.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start Savepoint window");
}
