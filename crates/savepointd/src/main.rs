#![windows_subsystem = "windows"]

//! savepointd: the always-on part of Savepoint. It has no window; it shows a
//! tray icon, notices when games close, and backs up their saves.
//!
//! `savepointd --scan` is the same binary run as a short-lived child process
//! to parse the game manifest, so that memory is returned to the system as
//! soon as the scan ends instead of staying in the service.

mod i18n;
mod log;
mod server;
mod state;
mod tray;
mod watcher;
mod win;

use log::log;
use savepoint_core::{installs, manifest, paths, scan};
use savepoint_ipc::Request;
use std::thread;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data_dir = paths::data_dir();
    log::init(data_dir.join("savepoint.log"));

    if args.iter().any(|a| a == "--scan") {
        std::process::exit(match scan_once(&data_dir) {
            Ok(()) => 0,
            Err(e) => {
                log!("scan error: {e}");
                1
            }
        });
    }

    if !win::acquire_single_instance() {
        // Already running: ask that instance to show its window instead.
        win::allow_foreground();
        let _ = savepoint_ipc::request(&Request::Show);
        return;
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    win::efficiency_mode();
    log!("savepointd {} started", env!("CARGO_PKG_VERSION"));

    let (shared, first_run) = state::Shared::start();
    let watcher = shared.clone();
    thread::Builder::new().name("watcher".into()).spawn(move || watcher::run(watcher)).expect("spawn watcher");
    let server = shared.clone();
    thread::Builder::new().name("pipe".into()).spawn(move || server::run(server)).expect("spawn pipe server");

    let from_ui = args.iter().any(|a| a == "--from-ui");
    if (first_run && !from_ui) || args.iter().any(|a| a == "--show") {
        win::open_ui();
    }
    tray::run(shared);
    log!("savepointd stopped");
}

fn scan_once(data_dir: &std::path::Path) -> std::io::Result<()> {
    win::background_process();
    let started = std::time::Instant::now();
    if let Err(e) = manifest::update(data_dir) {
        // A stale manifest is still useful; only fail if there is none at all.
        log!("manifest update failed: {e}");
    }
    let entries = manifest::load_index(data_dir)?;
    let folders = paths::UserFolders::detect();
    let libs = paths::steam_libraries();
    let others = installs::discover(&folders);
    let games = scan::detect(&entries, &folders, &libs, &others);
    scan::save_games(data_dir, &games)?;
    log!(
        "scan: {} manifest entries, {} steam libraries, {} other installs, {} games found in {:?}",
        entries.len(),
        libs.len(),
        others.len(),
        games.len(),
        started.elapsed()
    );
    Ok(())
}
