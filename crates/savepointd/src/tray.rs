//! Notification-area icon and its menu. Runs the main thread's message loop.

use crate::state::{Job, Shared};
use crate::{watcher, win};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowThreadProcessId,
    HICON, LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_APP, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{HSTRING, w};

const WM_TRAY: u32 = WM_APP + 1;
/// Posted to the tray window: wparam 1 shows the icon, 0 hides it.
const WM_SET_ICON: u32 = WM_APP + 2;
/// Posted to the tray window: ends the message loop, which stops the service.
const WM_QUIT_SERVICE: u32 = WM_APP + 3;
const ID_OPEN: usize = 1;
const ID_BACKUP_ALL: usize = 2;
const ID_QUIT: usize = 3;

static TRAY_ICO: &[u8] = include_bytes!("../assets/tray.ico");
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static ICON_VISIBLE: AtomicBool = AtomicBool::new(false);
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);

/// Shows or hides the icon; the service keeps running either way.
pub fn set_visible(visible: bool) {
    post(WM_SET_ICON, visible as usize);
}

/// Stops the whole service.
pub fn quit() {
    post(WM_QUIT_SERVICE, 0);
}

fn post(msg: u32, wparam: usize) {
    let hwnd = TRAY_HWND.load(Ordering::SeqCst);
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), msg, WPARAM(wparam), LPARAM(0));
        }
    }
}

pub fn run(shared: Arc<Shared>) {
    let show_tray = shared.show_tray();
    let _ = SHARED.set(shared);
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("SavepointTray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassW(&wc);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Savepoint service"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        TRAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);
        if show_tray {
            add_icon(hwnd);
        }
        // Delivered through this thread's message loop; nothing is injected
        // into other processes.
        let _hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        remove_icon(hwnd);
    }
}

fn notify_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() }
}

fn add_icon(hwnd: HWND) {
    ICON_VISIBLE.store(true, Ordering::SeqCst);
    let mut data = notify_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = cached_icon();
    for (dst, src) in data.szTip.iter_mut().zip("Savepoint".encode_utf16()) {
        *dst = src;
    }
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

fn remove_icon(hwnd: HWND) {
    ICON_VISIBLE.store(false, Ordering::SeqCst);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &notify_data(hwnd));
    }
}

fn cached_icon() -> HICON {
    let raw = ICON.load(Ordering::SeqCst);
    if raw != 0 {
        return HICON(raw as *mut _);
    }
    let icon = load_icon();
    ICON.store(icon.0 as isize, Ordering::SeqCst);
    icon
}

/// Picks the best-fitting image out of the embedded .ico for the current DPI.
fn load_icon() -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let ico = TRAY_ICO;
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut best: Option<(i32, usize, usize)> = None;
    for i in 0..count {
        let e = &ico[6 + 16 * i..6 + 16 * (i + 1)];
        let width = if e[0] == 0 { 256 } else { e[0] as i32 };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let better = match best {
            None => true,
            Some((w, _, _)) => (width >= size && (w < size || width < w)) || (w < size && width > w),
        };
        if better {
            best = Some((width, offset, len));
        }
    }
    let Some((_, offset, len)) = best else { return HICON::default() };
    unsafe {
        CreateIconFromResourceEx(&ico[offset..offset + len], true, 0x0003_0000, size, size, LR_DEFAULTCOLOR)
            .unwrap_or_default()
    }
}

fn show_menu(hwnd: HWND) -> usize {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let lang = SHARED.get().map_or("en", |s| s.language());
        let text = |key: &str| HSTRING::from(crate::i18n::tray(lang, key));
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN, &text("open"));
        let _ = AppendMenuW(menu, MF_STRING, ID_BACKUP_ALL, &text("backup_all"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, &text("quit"));
        let _ = SetMenuDefaultItem(menu, ID_OPEN as u32, 0);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // Without this the menu would not close when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, point.x, point.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as usize
    }
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    watcher::foreground_changed(pid);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_TRAY {
        match lparam.0 as u32 {
            WM_LBUTTONUP => win::open_ui(),
            WM_RBUTTONUP => match show_menu(hwnd) {
                ID_OPEN => win::open_ui(),
                ID_BACKUP_ALL => {
                    if let Some(shared) = SHARED.get() {
                        shared.queue(Job::BackupAll);
                    }
                }
                ID_QUIT => {
                    win::close_ui();
                    unsafe { PostQuitMessage(0) };
                }
                _ => {}
            },
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == WM_SET_ICON {
        match (wparam.0 != 0, ICON_VISIBLE.load(Ordering::SeqCst)) {
            (true, false) => add_icon(hwnd),
            (false, true) => remove_icon(hwnd),
            _ => {}
        }
        return LRESULT(0);
    }
    if msg == WM_QUIT_SERVICE {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) && ICON_VISIBLE.load(Ordering::SeqCst) {
        // Explorer restarted: the icon has to be added again.
        add_icon(hwnd);
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
