//! Wire protocol between the UI and `savepointd`.
//!
//! One request per connection: the client writes a single JSON line, the
//! server answers with a single JSON line and closes the pipe.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::thread;
use std::time::{Duration, Instant};

/// The pipe is per Windows user, so two people signed in at once don't clash.
pub fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!(r"\\.\pipe\savepoint-{user}")
}

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Status,
    Game {
        game: String,
    },
    BackupNow {
        game: Option<String>,
    },
    Restore {
        game: String,
        backup: String,
    },
    OpenBackups {
        game: Option<String>,
    },
    OpenSaves {
        game: String,
    },
    SetSettings {
        backup_dir: Option<String>,
        keep: Option<u32>,
        autostart: Option<bool>,
        show_tray: Option<bool>,
        lang: Option<String>,
    },
    AddGame {
        name: String,
        path: String,
        exe: Option<String>,
    },
    RemoveGame {
        game: String,
    },
    ResetHidden,
    /// Brings one hidden game back into the list.
    Unhide {
        game: String,
    },
    Rescan,
    /// What a restore would do, so the panel can warn before it happens.
    RestorePreview {
        game: String,
        backup: String,
    },
    /// Dismisses backup errors: one game's, or all of them.
    ClearErrors {
        game: Option<String>,
    },
    Show,
    /// Sent by the panel as it opens; the service may refresh a stale game list.
    UiOpened,
    /// Stops the service completely.
    Quit,
    /// The game's icon as a base64 PNG.
    Icon {
        game: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok(data: impl Serialize) -> Self {
        Reply { ok: true, data: serde_json::to_value(data).unwrap_or_default(), error: None }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Reply { ok: false, data: serde_json::Value::Null, error: Some(message.into()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub name: String,
    pub custom: bool,
    pub running: bool,
    pub has_icon: bool,
    /// False for games that only exist as backups here (e.g. made on another PC).
    pub local: bool,
    pub last_backup: Option<i64>,
    pub backups: u32,
    pub files: u32,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub id: String,
    pub time: i64,
    pub reason: String,
    pub files: u32,
    pub size: u64,
    /// "empty_file" or "much_smaller" when the saves looked damaged.
    #[serde(default)]
    pub suspect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameDetail {
    pub info: GameInfo,
    pub locations: Vec<String>,
    pub backups: Vec<BackupInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub games: Vec<GameInfo>,
    pub backup_dir: String,
    pub keep: u32,
    pub autostart: bool,
    pub show_tray: bool,
    /// "auto" or a language code such as "en".
    pub lang: String,
    pub errors: Vec<ErrorInfo>,
    pub scanning: bool,
    pub last_scan: Option<i64>,
    pub busy: Option<Busy>,
    pub hidden: u32,
    pub hidden_games: Vec<String>,
    /// The service's version, shown in Settings for bug reports.
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupResult {
    pub created: bool,
    /// "created", "unchanged" or "no_files"; the panel turns it into text.
    pub code: String,
}

/// What the service is working on right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Busy {
    /// "backup" or "restore".
    pub kind: String,
    pub game: String,
}

/// A problem the user should know about, such as a backup that failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub game: Option<String>,
    /// Stable code ("disk_full", "folder_unreachable", ...); the panel translates it.
    pub code: String,
    /// The system's own message, shown as a detail.
    pub detail: String,
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preview {
    pub files: u32,
    /// Files on disk that are newer than the copy in this backup.
    pub newer: u32,
}

/// True when the error means no server is listening.
pub fn is_not_running(err: &io::Error) -> bool {
    err.raw_os_error() == Some(ERROR_FILE_NOT_FOUND)
}

pub fn request(req: &Request) -> io::Result<Reply> {
    let line = serde_json::to_string(req)?;
    let reply = request_raw(&line)?;
    Ok(serde_json::from_str(&reply)?)
}

/// Sends one JSON line and returns the raw JSON reply line.
pub fn request_raw(line: &str) -> io::Result<String> {
    let file = connect(Duration::from_secs(3))?;
    let mut writer = &file;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    let mut reply = String::new();
    BufReader::new(&file).read_line(&mut reply)?;
    if reply.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "empty reply"));
    }
    Ok(reply)
}

/// Waits until the server accepts connections, for use right after spawning it.
pub fn wait_until_ready(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if request(&Request::Status).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

fn connect(timeout: Duration) -> io::Result<File> {
    let deadline = Instant::now() + timeout;
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe_name()) {
            Ok(file) => return Ok(file),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}
