//! Named-pipe server for the UI: one JSON line in, one JSON line out.

use crate::log::log;
use crate::state::{Job, Shared};
use crate::tray;
use crate::win;
use savepoint_core::backup::Outcome;
use savepoint_ipc::{self as ipc, Reply, Request};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::HSTRING;

const MAX_REQUEST: u64 = 1024 * 1024;

pub fn run(shared: Arc<Shared>) {
    let name = HSTRING::from(ipc::pipe_name());
    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            log!("CreateNamedPipe failed: {}", io::Error::last_os_error());
            thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if let Err(e) = connected
            && e.code() != ERROR_PIPE_CONNECTED.to_hresult()
        {
            unsafe { drop(File::from_raw_handle(pipe.0)) };
            continue;
        }
        // Each client gets its own thread so a slow restore never blocks status updates.
        let shared = shared.clone();
        let raw = pipe.0 as usize;
        thread::spawn(move || serve(&shared, HANDLE(raw as *mut _)));
    }
}

fn serve(shared: &Shared, pipe: HANDLE) {
    let file = unsafe { File::from_raw_handle(pipe.0) };
    let mut line = String::new();
    // A request is one short line; never read more than this from a client.
    // A valid request is one line ending in a newline. Anything else (cut off at the size limit, or the client
    // left) gets no reply: replying would wait for a client that is not reading.
    if BufReader::new((&file).take(MAX_REQUEST)).read_line(&mut line).is_err() || !line.ends_with('\n') {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(request) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(shared, request)))
            .unwrap_or_else(|_| {
                log!("a request panicked");
                Reply::err("internal_error")
            }),
        Err(e) => Reply::err(format!("bad request: {e}")),
    };
    let mut writer = &file;
    let mut out = serde_json::to_vec(&reply).unwrap_or_default();
    out.push(b'\n');
    let _ = writer.write_all(&out);
    let _ = file.sync_all();
}

fn handle(shared: &Shared, request: Request) -> Reply {
    let result: io::Result<Reply> = match request {
        Request::Status => Ok(Reply::ok(shared.status())),
        Request::Game { game } => shared.game_detail(&game).map(Reply::ok),
        Request::BackupNow { game: None } => {
            shared.queue(Job::BackupAll);
            Ok(Reply::ok(()))
        }
        Request::BackupNow { game: Some(game) } => shared.run_backup(&game, "manual").map(|outcome| {
            let (created, code) = match outcome {
                Outcome::Created(_) => (true, "created"),
                Outcome::Unchanged => (false, "unchanged"),
                Outcome::NoFiles => (false, "no_files"),
            };
            Reply::ok(ipc::BackupResult { created, code: code.into() })
        }),
        Request::Restore { game, backup } => shared.restore(&game, &backup).map(|()| Reply::ok(())),
        Request::OpenBackups { game } => shared.backups_folder(game.as_deref()).map(|dir| open_folder(&dir)),
        Request::OpenSaves { game } => shared.saves_folder(&game).map(|dir| open_folder(&dir)),
        Request::SetSettings { backup_dir, keep, autostart, show_tray, lang } => {
            shared.set_settings(backup_dir, keep, autostart, show_tray, lang).map(|()| Reply::ok(()))
        }
        Request::AddGame { name, path, exe } => shared.add_game(name, path, exe).map(|()| Reply::ok(())),
        Request::RemoveGame { game } => shared.remove_game(&game).map(|()| Reply::ok(())),
        Request::Unhide { game } => shared.unhide(&game).map(|()| Reply::ok(())),
        Request::ResetHidden => shared.reset_hidden().map(|()| Reply::ok(())),
        Request::Rescan => {
            shared.rescan();
            Ok(Reply::ok(()))
        }
        Request::RestorePreview { game, backup } => shared.restore_preview(&game, &backup).map(Reply::ok),
        Request::ClearErrors { game } => {
            shared.dismiss_errors(game.as_deref());
            Ok(Reply::ok(()))
        }
        Request::Show => {
            win::open_ui();
            Ok(Reply::ok(()))
        }
        Request::Icon { game } => shared.icon_data(&game).map(Reply::ok),
        Request::UiOpened => {
            shared.ui_opened();
            Ok(Reply::ok(()))
        }
        Request::Quit => {
            win::close_ui();
            tray::quit();
            Ok(Reply::ok(()))
        }
    };
    result.unwrap_or_else(|e| Reply::err(e.to_string()))
}

fn open_folder(dir: &Path) -> Reply {
    let _ = Command::new("explorer.exe").arg(dir).spawn();
    Reply::ok(())
}
