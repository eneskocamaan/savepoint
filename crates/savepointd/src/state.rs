//! Service state shared by the tray, watcher, pipe server and worker threads.
//!
//! `state` is only held for quick reads and writes; disk work happens outside
//! it, serialized by `io` so a backup and a restore never overlap.

use crate::log::log;
use crate::watcher::Matcher;
use crate::win;
use savepoint_core::backup::{self, Outcome, Summary};
use savepoint_core::{Config, CustomGame, Game, fsutil, icon, now_unix, paths, scan};
use savepoint_ipc as ipc;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

const RESCAN_AFTER: i64 = 7 * 24 * 60 * 60;
/// Opening the panel refreshes the game list when it is older than this.
const UI_RESCAN_AFTER: i64 = 10 * 60;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub enum Job {
    /// `settle`: wait for the game to finish writing before reading its saves.
    Backup {
        game: String,
        reason: &'static str,
        settle: bool,
    },
    BackupAll,
    Scan,
}

pub struct Shared {
    state: Mutex<State>,
    io: Mutex<()>,
    jobs: Mutex<Sender<Job>>,
    pub data_dir: PathBuf,
}

struct State {
    config: Config,
    detected: Vec<Game>,
    games: Vec<Game>,
    summaries: HashMap<String, Summary>,
    /// Games that have an icon file in the data folder.
    icons: HashSet<String>,
    running: HashSet<String>,
    scanning: bool,
    last_scan: Option<i64>,
    busy: Option<ipc::Busy>,
    errors: Vec<ipc::ErrorInfo>,
    matcher: Arc<Matcher>,
}

impl State {
    fn find(&self, name: &str) -> Option<&Game> {
        self.games.iter().find(|g| g.name == name)
    }

    fn info(&self, game: &Game) -> ipc::GameInfo {
        let summary = self.summaries.get(&game.name);
        let latest = summary.and_then(|s| s.latest.as_ref());
        ipc::GameInfo {
            name: game.name.clone(),
            custom: game.custom,
            running: self.running.contains(&game.name),
            has_icon: self.icons.contains(&game.name),
            local: !game.patterns.is_empty(),
            last_backup: latest.map(|b| b.time),
            backups: summary.map(|s| s.count).unwrap_or(0),
            files: latest.map(|b| b.files).unwrap_or(0),
            size: latest.map(|b| b.size).unwrap_or(0),
        }
    }
}

impl Shared {
    /// Loads saved state and starts the worker thread.
    /// Returns the shared state and whether this is the first run.
    pub fn start() -> (Arc<Shared>, bool) {
        let data_dir = paths::data_dir();
        let (config, existed) = Config::load();
        if !existed {
            let _ = config.save();
        }
        let detected = scan::load_games(&data_dir);
        let last_scan = fsutil::mtime_unix(&scan::games_file(&data_dir));
        let needs_scan = detected.is_none() || last_scan.is_none_or(|t| now_unix() - t > RESCAN_AFTER);

        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                config,
                detected: detected.unwrap_or_default(),
                games: Vec::new(),
                summaries: HashMap::new(),
                icons: HashSet::new(),
                running: HashSet::new(),
                scanning: needs_scan,
                last_scan,
                busy: None,
                errors: Vec::new(),
                matcher: Arc::new(Matcher::default()),
            }),
            io: Mutex::new(()),
            jobs: Mutex::new(tx),
            data_dir,
        });
        shared.rebuild(true);
        if needs_scan {
            shared.queue(Job::Scan);
        }
        let worker = shared.clone();
        thread::Builder::new().name("worker".into()).spawn(move || worker.work(rx)).expect("spawn worker");
        (shared, !existed)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn queue(&self, job: Job) {
        let _ = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).send(job);
    }

    /// The panel just opened: refresh the game list if it has gone stale, so a
    /// game installed a minute ago shows up without pressing anything.
    pub fn ui_opened(&self) {
        let stale = {
            let mut s = self.lock();
            let stale = !s.scanning && s.last_scan.is_none_or(|t| now_unix() - t > UI_RESCAN_AFTER);
            s.scanning |= stale;
            stale
        };
        if stale {
            self.queue(Job::Scan);
        }
    }

    /// The language the service itself uses (tray menu), with "auto" resolved.
    pub fn language(&self) -> &'static str {
        crate::i18n::resolve(&self.lock().config.lang)
    }

    pub fn show_tray(&self) -> bool {
        self.lock().config.show_tray
    }

    pub fn matcher(&self) -> Arc<Matcher> {
        self.lock().matcher.clone()
    }

    /// Recomputes the game list after a scan or a config change.
    fn rebuild(&self, reload_summaries: bool) {
        // Games with backups here, like ones made on another PC, are listed even if not installed.
        let backups_root = PathBuf::from(&self.lock().config.backup_dir);
        let in_backups = backup::discover(&backups_root);
        let (root, missing) = {
            let mut s = self.lock();
            let custom: Vec<Game> = s.config.custom.iter().map(CustomGame::to_game).collect();
            let mut games: Vec<Game> = s
                .detected
                .iter()
                .filter(|g| !s.config.hidden.contains(&g.name))
                .filter(|g| !custom.iter().any(|c| c.name.eq_ignore_ascii_case(&g.name)))
                .cloned()
                .collect();
            games.extend(custom);
            for name in in_backups {
                let known = games.iter().any(|g| g.name.eq_ignore_ascii_case(&name));
                if !known && !s.config.hidden.contains(&name) {
                    games.push(Game { name, ..Default::default() });
                }
            }
            if reload_summaries {
                s.summaries.clear();
            }
            let missing: Vec<Game> = games.iter().filter(|g| !s.summaries.contains_key(&g.name)).cloned().collect();
            s.matcher = Arc::new(Matcher::new(&games));
            s.games = games;
            (PathBuf::from(&s.config.backup_dir), missing)
        };
        let fresh: Vec<(String, Summary)> =
            missing.iter().map(|g| (g.name.clone(), backup::summary(&root, g))).collect();
        let names: Vec<String> = self.lock().games.iter().map(|g| g.name.clone()).collect();
        let icons: HashSet<String> = names.into_iter().filter(|n| icon::path(&self.data_dir, n).is_file()).collect();
        let mut s = self.lock();
        s.summaries.extend(fresh);
        s.icons = icons;
        drop(s);
        crate::watcher::wake();
    }

    fn refresh_summary(&self, game: &Game, root: &Path) {
        let summary = backup::summary(root, game);
        self.lock().summaries.insert(game.name.clone(), summary);
    }

    fn set_busy(&self, kind: &str, game: Option<&str>) {
        self.lock().busy = game.map(|game| ipc::Busy { kind: kind.to_string(), game: game.to_string() });
    }

    // ---- errors the user should see ----

    /// Records a problem for a game (or a general one when `game` is `None`).
    /// A game has at most one entry; a newer problem replaces the older one.
    fn report_error(&self, game: Option<&str>, code: &str, detail: String) {
        let mut s = self.lock();
        s.errors.retain(|e| e.game.as_deref() != game);
        s.errors.push(ipc::ErrorInfo {
            game: game.map(String::from),
            code: code.to_string(),
            detail,
            time: now_unix(),
        });
    }

    fn clear_error(&self, game: Option<&str>) {
        self.lock().errors.retain(|e| e.game.as_deref() != game);
    }

    /// The panel's dismiss button: one game's error, or every error.
    pub fn dismiss_errors(&self, game: Option<&str>) {
        match game {
            Some(game) => self.clear_error(Some(game)),
            None => self.lock().errors.clear(),
        }
    }

    fn record(&self, name: &str, result: &io::Result<Outcome>) {
        match result {
            Ok(Outcome::Created(b)) if b.meta.suspect.is_some() => {
                let why = b.meta.suspect.clone().unwrap_or_default();
                self.report_error(Some(name), "suspect_save", why);
            }
            Ok(Outcome::Created(_)) => self.clear_error(Some(name)),
            // Nothing new was saved, so a damaged-save warning is still true.
            Ok(Outcome::Unchanged) => {
                let warned =
                    self.lock().errors.iter().any(|e| e.game.as_deref() == Some(name) && e.code == "suspect_save");
                if !warned {
                    self.clear_error(Some(name));
                }
            }
            Ok(Outcome::NoFiles) => {}
            Err(e) => {
                let (code, detail) = classify(e);
                self.report_error(Some(name), code, detail);
            }
        }
    }

    fn snapshot(&self, name: &str) -> io::Result<(Game, PathBuf, u32)> {
        let s = self.lock();
        let game = s.find(name).cloned().ok_or_else(|| io::Error::other("game_not_found"))?;
        Ok((game, PathBuf::from(&s.config.backup_dir), s.config.keep))
    }

    // ---- icons ----

    /// Saves the exe's icon for a game unless it already has one.
    pub fn learn_icon(&self, game: &str, exe: &Path) {
        if self.lock().icons.contains(game) {
            return;
        }
        match icon::save_from_exe(&self.data_dir, game, exe) {
            Ok(true) => {
                self.lock().icons.insert(game.to_string());
            }
            Ok(false) => {}
            Err(e) => log!("icon for {game} failed: {e}"),
        }
    }

    pub fn icon_data(&self, game: &str) -> io::Result<String> {
        icon::read_base64(&self.data_dir, game)
    }

    // ---- watcher events ----

    pub fn game_started(&self, name: &str) {
        log!("started: {name}");
        self.lock().running.insert(name.to_string());
    }

    pub fn game_stopped(&self, name: &str) {
        log!("stopped: {name}");
        self.lock().running.remove(name);
        self.queue(Job::Backup { game: name.to_string(), reason: "auto", settle: true });
    }

    // ---- operations ----

    /// One backup attempt that records nothing; callers decide what a failure means.
    fn backup_once(&self, name: &str, reason: &str) -> io::Result<Outcome> {
        let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let (game, root, keep) = self.snapshot(name)?;
        self.set_busy("backup", Some(name));
        // A missing backup folder (unplugged drive, signed-out cloud folder) is
        // the most common reason for a backup to fail, so check it first.
        let result = std::fs::create_dir_all(&root).and_then(|()| backup::backup(&root, &game, keep, reason, None));
        self.set_busy("backup", None);
        self.refresh_summary(&game, &root);
        match &result {
            Ok(Outcome::Created(b)) => log!("backup {name}: {} ({} files)", b.id, b.meta.files.len()),
            Ok(other) => log!("backup {name}: {other:?}"),
            Err(e) => log!("backup {name} failed: {e}"),
        }
        result
    }

    pub fn run_backup(&self, name: &str, reason: &str) -> io::Result<Outcome> {
        let result = self.backup_once(name, reason);
        self.record(name, &result);
        result
    }

    /// Backup after a game closed. Antivirus, launchers and cloud sync often
    /// hold a save for a few seconds, so a locked file gets a few more tries.
    fn backup_after_exit(&self, name: &str) {
        let mut result = self.backup_once(name, "auto");
        for _ in 0..3 {
            match &result {
                Err(e) if matches!(classify(e).0, "file_locked" | "access_denied") => {
                    thread::sleep(Duration::from_secs(6));
                    result = self.backup_once(name, "auto");
                }
                _ => break,
            }
        }
        self.record(name, &result);
    }

    /// Starts a scan right away from the user's point of view: the flag is
    /// raised now, so the panel shows progress without waiting for the worker.
    pub fn rescan(&self) {
        let start = {
            let mut s = self.lock();
            !std::mem::replace(&mut s.scanning, true)
        };
        if start {
            self.queue(Job::Scan);
        }
    }

    pub fn restore_preview(&self, name: &str, id: &str) -> io::Result<ipc::Preview> {
        let (game, root, _) = self.snapshot(name)?;
        let p = backup::preview(&root, &game, id)?;
        Ok(ipc::Preview { files: p.files, newer: p.newer })
    }

    pub fn restore(&self, name: &str, id: &str) -> io::Result<()> {
        let _io = self.io.lock().unwrap_or_else(|e| e.into_inner());
        let (game, root, keep) = self.snapshot(name)?;
        if self.lock().running.contains(name) {
            return Err(io::Error::other("game_running"));
        }
        self.set_busy("restore", Some(name));
        let result = backup::restore(&root, &game, id, keep);
        self.set_busy("restore", None);
        self.refresh_summary(&game, &root);
        match &result {
            Ok(()) => log!("restore {name}: {id}"),
            Err(e) => log!("restore {name} {id} failed: {e}"),
        }
        result
    }

    fn run_scan(&self) {
        self.lock().scanning = true;
        log!("scan started");
        let exe = std::env::current_exe().unwrap_or_default();
        let status = Command::new(exe).arg("--scan").creation_flags(CREATE_NO_WINDOW).status();
        match status {
            Ok(s) if s.success() => {
                if let Some(games) = scan::load_games(&self.data_dir) {
                    log!("scan finished: {} games", games.len());
                    self.clear_error(None);
                    let mut s = self.lock();
                    s.detected = games;
                    s.last_scan = Some(now_unix());
                }
            }
            other => {
                log!("scan failed: {other:?}");
                self.report_error(None, "scan_failed", format!("{other:?}"));
            }
        }
        self.rebuild(false);
        self.lock().scanning = false;
    }

    fn work(&self, jobs: Receiver<Job>) {
        win::background_thread();
        for job in jobs {
            // One bad job must not take the worker, and with it every backup, down.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_job(job))).is_err() {
                log!("a background job panicked");
                let mut s = self.lock();
                s.busy = None;
                s.scanning = false;
            }
        }
    }

    fn run_job(&self, job: Job) {
        match job {
            Job::Backup { game, reason, settle } => {
                if settle {
                    self.wait_settle(&game);
                }
                if reason == "auto" {
                    self.backup_after_exit(&game);
                } else {
                    let _ = self.run_backup(&game, reason);
                }
            }
            Job::BackupAll => {
                let names: Vec<String> = self.lock().games.iter().map(|g| g.name.clone()).collect();
                for name in names {
                    let _ = self.run_backup(&name, "manual");
                }
            }
            Job::Scan => self.run_scan(),
        }
    }

    /// Games often keep writing for a moment after their process exits.
    fn wait_settle(&self, name: &str) {
        let Ok((game, _, _)) = self.snapshot(name) else { return };
        thread::sleep(Duration::from_secs(3));
        let mut before = game.collect_files();
        for _ in 0..10 {
            thread::sleep(Duration::from_secs(2));
            let after = game.collect_files();
            if after == before {
                return;
            }
            before = after;
        }
    }

    // ---- pipe requests ----

    pub fn status(&self) -> ipc::Status {
        let s = self.lock();
        ipc::Status {
            games: s.games.iter().map(|g| s.info(g)).collect(),
            backup_dir: s.config.backup_dir.clone(),
            keep: s.config.keep,
            autostart: win::autostart_enabled(),
            show_tray: s.config.show_tray,
            lang: s.config.lang.clone(),
            errors: s.errors.clone(),
            scanning: s.scanning,
            last_scan: s.last_scan,
            busy: s.busy.clone(),
            hidden: s.config.hidden.len() as u32,
            hidden_games: s.config.hidden.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn game_detail(&self, name: &str) -> io::Result<ipc::GameDetail> {
        let (game, root, _) = self.snapshot(name)?;
        let info = self.lock().info(&game);
        let backups = backup::list(&root, &game)
            .into_iter()
            .map(|b| ipc::BackupInfo {
                size: b.size(),
                files: b.meta.files.len() as u32,
                time: b.meta.time,
                reason: b.meta.reason,
                suspect: b.meta.suspect,
                id: b.id,
            })
            .collect();
        Ok(ipc::GameDetail { info, locations: game.locations(), backups })
    }

    pub fn backups_folder(&self, name: Option<&str>) -> io::Result<PathBuf> {
        let root = PathBuf::from(&self.lock().config.backup_dir);
        let dir = match name {
            Some(name) => backup::game_dir(&root, &self.snapshot(name)?.0),
            None => root,
        };
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn saves_folder(&self, name: &str) -> io::Result<PathBuf> {
        let (game, _, _) = self.snapshot(name)?;
        let first = game.collect_files().into_iter().next();
        let dir = first
            .and_then(|f| f.path.parent().map(Path::to_path_buf))
            .or_else(|| game.locations().first().map(PathBuf::from))
            .filter(|p| p.is_dir())
            .ok_or_else(|| io::Error::other("no_save_folder"))?;
        Ok(dir)
    }

    pub fn set_settings(
        &self,
        backup_dir: Option<String>,
        keep: Option<u32>,
        autostart: Option<bool>,
        show_tray: Option<bool>,
        lang: Option<String>,
    ) -> io::Result<()> {
        if let Some(enabled) = autostart {
            win::set_autostart(enabled)?;
        }
        // Creating the folder can be slow on a network or cloud drive, so it is
        // done before taking the lock that every status request needs.
        let backup_dir = backup_dir.filter(|d| !d.trim().is_empty());
        if let Some(dir) = &backup_dir {
            std::fs::create_dir_all(dir)?;
        }
        let dir_changed = {
            let mut s = self.lock();
            let mut changed = false;
            if let Some(dir) = backup_dir {
                changed = dir != s.config.backup_dir;
                s.config.backup_dir = dir;
            }
            if let Some(keep) = keep {
                s.config.keep = keep.clamp(1, 1000);
            }
            if let Some(show) = show_tray {
                s.config.show_tray = show;
            }
            if let Some(lang) = lang.filter(|l| crate::i18n::is_supported(l)) {
                s.config.lang = lang;
            }
            s.config.save()?;
            changed
        };
        if let Some(show) = show_tray {
            crate::tray::set_visible(show);
        }
        if dir_changed {
            self.rebuild(true);
        }
        Ok(())
    }

    pub fn add_game(&self, name: String, path: String, exe: Option<String>) -> io::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(io::Error::other("name_empty"));
        }
        if !Path::new(&path).exists() {
            return Err(io::Error::other("folder_missing"));
        }
        let exe = exe.filter(|e| !e.trim().is_empty());
        if exe.as_ref().is_some_and(|e| !Path::new(e).is_file()) {
            return Err(io::Error::other("exe_missing"));
        }
        {
            let mut s = self.lock();
            if s.config.custom.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
                return Err(io::Error::other("name_exists"));
            }
            s.config.custom.push(CustomGame { name: name.clone(), path, exe: exe.clone() });
            s.config.save()?;
        }
        if let Some(exe) = exe {
            let _ = icon::save_from_exe(&self.data_dir, &name, Path::new(&exe));
        }
        self.rebuild(false);
        Ok(())
    }

    pub fn remove_game(&self, name: &str) -> io::Result<()> {
        {
            let mut s = self.lock();
            let before = s.config.custom.len();
            s.config.custom.retain(|c| c.name != name);
            if s.config.custom.len() == before && !s.config.hidden.iter().any(|h| h == name) {
                s.config.hidden.push(name.to_string());
            }
            s.summaries.remove(name);
            s.config.save()?;
        }
        self.rebuild(false);
        Ok(())
    }

    pub fn unhide(&self, name: &str) -> io::Result<()> {
        {
            let mut s = self.lock();
            s.config.hidden.retain(|h| h != name);
            s.config.save()?;
        }
        self.rebuild(false);
        Ok(())
    }

    pub fn reset_hidden(&self) -> io::Result<()> {
        {
            let mut s = self.lock();
            s.config.hidden.clear();
            s.config.save()?;
        }
        self.rebuild(false);
        Ok(())
    }
}

/// Turns an I/O failure into a stable code the panel can translate, plus the
/// system's own message as a detail.
fn classify(e: &io::Error) -> (&'static str, String) {
    // Windows error numbers: 112/39 disk full, 5 access denied, 32/33 file in use,
    // 2/3 not found, 21 device not ready, 53/67/1231 network path unavailable.
    let code = match e.raw_os_error() {
        Some(112 | 39) => "disk_full",
        Some(5) => "access_denied",
        Some(32 | 33) => "file_locked",
        Some(2 | 3 | 21 | 53 | 67 | 1231) => "folder_unreachable",
        _ => match e.kind() {
            io::ErrorKind::PermissionDenied => "access_denied",
            io::ErrorKind::NotFound => "folder_unreachable",
            io::ErrorKind::StorageFull => "disk_full",
            _ => "backup_failed",
        },
    };
    // Raised by the engine itself rather than by the system.
    let code = if e.to_string() == "too_large" { "too_large" } else { code };
    (code, e.to_string())
}
