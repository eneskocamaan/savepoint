//! Notices when a known game starts and stops, without polling.
//!
//! - Start: games take the foreground when they open, so the tray thread
//!   listens for foreground-window changes (`SetWinEventHook`) and hands the
//!   window's process id to this thread. If that is missed (a game that opens
//!   behind another window), a once-a-minute snapshot still catches it; that
//!   snapshot only inspects processes it has not seen before.
//! - Stop: for each running game process we keep a handle and block on it, so
//!   the exit is seen the moment it happens.
//!
//! Between those events the thread is asleep.
//! (ETW would see every process start too, but requires admin rights.)

use crate::state::Shared;
use savepoint_core::{Game, installs};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, SetEvent, WaitForMultipleObjects,
};
use windows::core::PWSTR;

/// WaitForMultipleObjects takes at most 64 handles; one is the wake event.
const MAX_TRACKED: usize = 63;
/// Safety-net snapshot interval, in milliseconds.
const SWEEP_MS: u32 = 60_000;

/// Process ids reported by the foreground hook, waiting to be checked.
static PENDING: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// Auto-reset event that wakes the watcher thread (stored as a raw value).
static WAKE: Mutex<usize> = Mutex::new(0);

/// Called from the tray thread when a window comes to the foreground.
pub fn foreground_changed(pid: u32) {
    if pid != 0 && pid != std::process::id() {
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
        wake();
    }
}

/// Makes the watcher re-check its state (used after the game list changes).
pub fn wake() {
    let raw = *WAKE.lock().unwrap_or_else(|e| e.into_inner());
    if raw != 0 {
        unsafe {
            let _ = SetEvent(HANDLE(raw as *mut _));
        }
    }
}

/// Maps process image paths to game names.
#[derive(Default)]
pub struct Matcher {
    /// Lowercase install folder (with trailing `\`) → game name.
    dirs: Vec<(String, String)>,
    /// Normalized folder name → game name, matched against every folder in the
    /// process path (Epic, GOG, repacks, cracked copies in any folder).
    names: HashMap<String, String>,
    /// Steam app id → game name, for cracked copies that carry a `steam_appid.txt`.
    steam_ids: HashMap<u64, String>,
    /// Lowercase exe path → game name.
    exes: HashMap<String, String>,
}

impl Matcher {
    pub fn new(games: &[Game]) -> Self {
        let mut m = Matcher::default();
        for game in games {
            for dir in &game.install_dirs {
                let mut dir = dir.to_lowercase().replace('/', "\\");
                if !dir.ends_with('\\') {
                    dir.push('\\');
                }
                m.dirs.push((dir, game.name.clone()));
            }
            for name in game.install_names.iter().chain(std::iter::once(&game.name)) {
                let name = installs::normalize_name(name);
                if !installs::is_generic_folder(&name) {
                    m.names.entry(name).or_insert_with(|| game.name.clone());
                }
            }
            if let Some(id) = game.steam_id {
                m.steam_ids.entry(id).or_insert_with(|| game.name.clone());
            }
            for exe in &game.exes {
                m.exes.insert(exe.to_lowercase().replace('/', "\\"), game.name.clone());
            }
        }
        // Longest prefix first, so nested install folders resolve to the right game.
        m.dirs.sort_by_key(|(d, _)| std::cmp::Reverse(d.len()));
        m
    }

    fn find(&self, image: &str) -> Option<&str> {
        let image = image.to_lowercase();
        if let Some(name) = self.exes.get(&image) {
            return Some(name);
        }
        if let Some((_, name)) = self.dirs.iter().find(|(dir, _)| image.starts_with(dir.as_str())) {
            return Some(name);
        }
        // Skip system folders, then look for a folder named like a known game.
        if image.contains(r"\windows\") {
            return None;
        }
        let folders = image.rsplit_once('\\').map(|(dir, _)| dir).unwrap_or("");
        folders.split('\\').find_map(|part| self.names.get(&installs::normalize_name(part))).map(String::as_str)
    }

    /// Cracked copies name the Steam game they pretend to be in a file next to the exe.
    fn find_by_appid(&self, image: &str) -> Option<&str> {
        if self.steam_ids.is_empty() || image.to_lowercase().contains(r"\windows\") {
            return None;
        }
        let dir = Path::new(image).parent()?;
        let id = {
            let mut cache = APPID_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            *cache.entry(dir.to_path_buf()).or_insert_with(|| installs::appid_near(dir))
        };
        self.steam_ids.get(&id?).map(String::as_str)
    }
}

/// App ids found next to exes, so each folder is inspected only once.
static APPID_CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<u64>>>> = LazyLock::new(Default::default);

struct Tracked {
    pid: u32,
    handle: HANDLE,
    game: String,
}

pub fn run(shared: Arc<Shared>) {
    let Ok(event) = (unsafe { CreateEventW(None, false, false, None) }) else { return };
    *WAKE.lock().unwrap_or_else(|e| e.into_inner()) = event.0 as usize;

    let mut tracked: Vec<Tracked> = Vec::new();
    // Processes already inspected by a sweep, so each sweep only looks at new ones.
    let mut seen: HashSet<u32> = HashSet::new();
    // Starts empty so the first pass always sweeps.
    let mut matcher = Arc::new(Matcher::default());
    let mut sweep = true;

    loop {
        let current = shared.matcher();
        if !Arc::ptr_eq(&current, &matcher) {
            matcher = current;
            seen.clear();
            sweep = true;
        }
        if sweep {
            let pids = snapshot_pids();
            for &pid in pids.iter().filter(|pid| !seen.contains(pid)) {
                check(pid, &matcher, &mut tracked, &shared);
            }
            seen = pids;
            sweep = false;
        }
        let pending: Vec<u32> = std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()));
        for pid in pending {
            check(pid, &matcher, &mut tracked, &shared);
        }

        let mut handles = Vec::with_capacity(tracked.len() + 1);
        handles.push(event);
        handles.extend(tracked.iter().map(|t| t.handle));
        let result = unsafe { WaitForMultipleObjects(&handles, false, SWEEP_MS) };
        let index = result.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if index == 0 {
            continue;
        }
        if index > tracked.len() {
            sweep = true;
            continue;
        }
        let gone = tracked.swap_remove(index - 1);
        unsafe {
            let _ = CloseHandle(gone.handle);
        }
        if !tracked.iter().any(|t| t.game == gone.game) {
            shared.game_stopped(&gone.game);
        }
    }
}

fn snapshot_pids() -> HashSet<u32> {
    let mut pids = HashSet::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return pids };
    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok {
        if entry.th32ProcessID > 4 && entry.th32ProcessID != std::process::id() {
            pids.insert(entry.th32ProcessID);
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    pids
}

fn check(pid: u32, matcher: &Matcher, tracked: &mut Vec<Tracked>, shared: &Shared) {
    if tracked.len() >= MAX_TRACKED || tracked.iter().any(|t| t.pid == pid) {
        return;
    }
    let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    let Ok(handle) = (unsafe { OpenProcess(access, false, pid) }) else { return };
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let named = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) };
    let image = named.ok().map(|_| String::from_utf16_lossy(&buf[..len as usize]));
    let game = image.as_deref().and_then(|image| matcher.find(image).or_else(|| matcher.find_by_appid(image)));
    match game {
        Some(name) => {
            if let Some(image) = &image {
                shared.learn_icon(name, std::path::Path::new(image));
            }
            let first = !tracked.iter().any(|t| t.game == name);
            tracked.push(Tracked { pid, handle, game: name.to_string() });
            if first {
                shared.game_started(name);
            }
        }
        None => unsafe {
            let _ = CloseHandle(handle);
        },
    }
}
