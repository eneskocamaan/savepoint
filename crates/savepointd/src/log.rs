//! Append-only log in the data folder, capped at 1 MB (one old copy is kept).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_SIZE: u64 = 1024 * 1024;

static LOG: Mutex<Option<(PathBuf, File)>> = Mutex::new(None);

pub fn init(path: PathBuf) {
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, file));
    }
}

pub fn write(message: &str) {
    let mut guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some((path, file)) = guard.as_mut() else { return };
    if file.metadata().is_ok_and(|m| m.len() > MAX_SIZE) {
        let _ = fs::rename(&*path, path.with_extension("old.log"));
        if let Ok(fresh) = OpenOptions::new().create(true).append(true).open(&*path) {
            *file = fresh;
        }
    }
    let _ = writeln!(file, "{} {message}", savepoint_core::win::local_timestamp());
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
pub(crate) use log;
