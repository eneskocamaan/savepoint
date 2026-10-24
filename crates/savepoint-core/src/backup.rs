//! Versioned backups as plain folders, so they can be browsed without Savepoint:
//!
//! ```text
//! <backup root>/<game>/20261021-195503/
//!     backup.json
//!     documents/Game/save.dat
//!     appdata/Game/settings.ini
//! ```
//!
//! Files are stored relative to well-known folders (see [`crate::portable`]),
//! which is what lets a backup made on one PC be restored on another.
//!
//! A backup is written under a `.tmp` name and renamed once complete, so a
//! crash never leaves a half-written backup behind. Files unchanged since the
//! previous backup are hard-linked to it instead of copied when possible.

use crate::game::{Game, SaveFile};
use crate::paths::UserFolders;
use crate::portable::{self, Locations};
use crate::{fsutil, now_unix, win};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const META_FILE: &str = "backup.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMeta {
    pub game: String,
    pub time: i64,
    /// "auto", "manual" or "pre-restore".
    pub reason: String,
    pub files: Vec<FileRecord>,
    /// Set when the saves looked damaged at backup time: "empty_file" or "much_smaller".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    /// Absolute path on the PC that made the backup (informational).
    pub path: String,
    /// Where the file is inside the backup, relative to a well-known folder.
    /// Missing in backups made by early versions, which used the drive layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored: Option<String>,
    pub size: u64,
    pub mtime: u64,
    pub hash: String,
}

impl FileRecord {
    fn stored_rel(&self) -> PathBuf {
        match &self.stored {
            Some(stored) => PathBuf::from(stored),
            None => portable::drive_stored(Path::new(&self.path)),
        }
    }

    fn key(&self, loc: &Locations) -> String {
        match &self.stored {
            Some(stored) => portable::key(Path::new(stored)),
            None => portable::key(&loc.to_stored(Path::new(&self.path))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Backup {
    pub id: String,
    pub meta: BackupMeta,
}

impl Backup {
    pub fn size(&self) -> u64 {
        self.meta.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug)]
pub enum Outcome {
    Created(Backup),
    Unchanged,
    NoFiles,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub count: u32,
    pub latest: Option<Latest>,
}

/// What the game list needs to know about a game's newest backup. Kept small on
/// purpose: the service holds one per game for as long as it runs.
#[derive(Debug, Clone)]
pub struct Latest {
    pub id: String,
    pub time: i64,
    pub files: u32,
    pub size: u64,
}

pub fn game_dir(root: &Path, game: &Game) -> PathBuf {
    root.join(game.folder_name())
}

fn backup_ids(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut ids: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.ends_with(".tmp"))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids
}

fn read_meta(dir: &Path) -> io::Result<BackupMeta> {
    let bytes = fs::read(dir.join(META_FILE))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// All backups of a game, newest first.
pub fn list(root: &Path, game: &Game) -> Vec<Backup> {
    let dir = game_dir(root, game);
    backup_ids(&dir)
        .into_iter()
        .filter_map(|id| read_meta(&dir.join(&id)).ok().map(|meta| Backup { id, meta }))
        .collect()
}

pub fn summary(root: &Path, game: &Game) -> Summary {
    let dir = game_dir(root, game);
    let ids = backup_ids(&dir);
    let latest = ids.iter().find_map(|id| {
        let meta = read_meta(&dir.join(id)).ok()?;
        Some(Latest {
            id: id.clone(),
            time: meta.time,
            files: meta.files.len() as u32,
            size: meta.files.iter().map(|f| f.size).sum(),
        })
    });
    Summary { count: ids.len() as u32, latest }
}

/// The newest backup with all its file records, for comparing against.
fn latest_backup(root: &Path, game: &Game) -> Option<Backup> {
    let dir = game_dir(root, game);
    backup_ids(&dir).into_iter().find_map(|id| read_meta(&dir.join(&id)).ok().map(|meta| Backup { id, meta }))
}

/// Names of the games that have backups in `root`, e.g. made on another PC.
pub fn discover(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else { return Vec::new() };
    let mut names = Vec::new();
    for entry in entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
        let dir = entry.path();
        if let Some(meta) = backup_ids(&dir).iter().find_map(|id| read_meta(&dir.join(id)).ok()) {
            names.push(meta.game);
        }
    }
    names.sort();
    names
}

/// Backs up the game's current saves unless they match the latest backup.
pub fn backup(root: &Path, game: &Game, keep: u32, reason: &str, protect: Option<&str>) -> io::Result<Outcome> {
    let loc = Locations::new(&UserFolders::detect(), game);
    backup_in(root, game, keep, reason, protect, &loc)
}

fn backup_in(
    root: &Path,
    game: &Game,
    keep: u32,
    reason: &str,
    protect: Option<&str>,
    loc: &Locations,
) -> io::Result<Outcome> {
    let files = game.collect_files();
    if files.is_empty() {
        return Ok(Outcome::NoFiles);
    }
    check_size(&files)?;
    let dir = game_dir(root, game);
    let previous = latest_backup(root, game);
    let known: HashMap<String, &FileRecord> =
        previous.as_ref().map(|b| b.meta.files.iter().map(|f| (f.key(loc), f)).collect()).unwrap_or_default();

    // Cheap check first: size + mtime. Only files that look different get hashed.
    let stored: Vec<PathBuf> = files.iter().map(|f| loc.to_stored(&f.path)).collect();
    let mut hashes: Vec<Option<String>> = Vec::with_capacity(files.len());
    let mut changed = previous.as_ref().is_none_or(|b| b.meta.files.len() != files.len());
    for (file, stored) in files.iter().zip(&stored) {
        match known.get(&portable::key(stored)) {
            Some(p) if p.size == file.size && p.mtime == file.mtime => hashes.push(Some(p.hash.clone())),
            Some(p) => match fsutil::hash_file(&file.path) {
                Ok(hash) => {
                    changed |= hash != p.hash;
                    hashes.push(Some(hash));
                }
                // Temporary saves come and go while a game runs.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    changed = true;
                    hashes.push(None);
                }
                Err(e) => return Err(e),
            },
            None => {
                changed = true;
                hashes.push(None);
            }
        }
    }
    if !changed {
        return Ok(Outcome::Unchanged);
    }

    fs::create_dir_all(&dir)?;
    let id = new_id(&dir);
    let tmp = dir.join(format!("{id}.tmp"));
    let _ = fs::remove_dir_all(&tmp);

    let previous_dir = previous.as_ref().map(|b| dir.join(&b.id));
    let written = write_files(&tmp, &files, &stored, &hashes, &known, previous_dir.as_deref());
    let records = match written {
        Ok(records) => records,
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    if records.is_empty() {
        let _ = fs::remove_dir_all(&tmp);
        return Ok(Outcome::NoFiles);
    }
    let suspect = assess(&files, &stored, &known, previous.as_ref());
    let meta =
        BackupMeta { game: game.name.clone(), time: now_unix(), reason: reason.to_string(), files: records, suspect };
    fsutil::write_atomic(&tmp.join(META_FILE), &serde_json::to_vec_pretty(&meta)?)?;
    fs::rename(&tmp, dir.join(&id))?;

    prune(&dir, keep, protect);
    Ok(Outcome::Created(Backup { id, meta }))
}

/// No real save set is this big. If a game's paths ever match an install
/// folder or a whole drive, backing it up would fill the disk, so refuse.
const MAX_BACKUP_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn check_size(files: &[SaveFile]) -> io::Result<()> {
    let total: u64 = files.iter().map(|f| f.size).sum();
    if total > MAX_BACKUP_BYTES { Err(io::Error::other("too_large")) } else { Ok(()) }
}

/// Games that crash or lose power mid-write leave empty or truncated saves.
/// Spots the two obvious signs, compared with the previous backup:
/// a file that had content and is now empty, or all saves together shrinking to
/// under a fifth of their size.
fn assess(
    files: &[SaveFile],
    stored: &[PathBuf],
    known: &HashMap<String, &FileRecord>,
    previous: Option<&Backup>,
) -> Option<String> {
    let previous = previous?;
    let emptied = files
        .iter()
        .zip(stored)
        .any(|(file, rel)| file.size == 0 && known.get(&portable::key(rel)).is_some_and(|p| p.size > 0));
    if emptied {
        return Some("empty_file".to_string());
    }
    let before = previous.size();
    let now: u64 = files.iter().map(|f| f.size).sum();
    (before >= SHRINK_MIN_BYTES && now * 5 < before).then(|| "much_smaller".to_string())
}

/// Below this, a size drop says nothing (tiny settings-like saves).
const SHRINK_MIN_BYTES: u64 = 4096;

/// A folder name that sorts after every existing backup, even when several
/// are taken within the same second: `20261021-195503`, `…-02`, `…-03`.
fn new_id(dir: &Path) -> String {
    let stamp = win::local_timestamp();
    let last = backup_ids(dir)
        .iter()
        .filter_map(|id| id.strip_prefix(stamp.as_str()))
        .map(|rest| rest.strip_prefix('-').and_then(|n| n.parse::<u32>().ok()).unwrap_or(1))
        .max();
    match last {
        None => stamp,
        Some(n) => format!("{stamp}-{:02}", n + 1),
    }
}

fn write_files(
    tmp: &Path,
    files: &[SaveFile],
    stored: &[PathBuf],
    hashes: &[Option<String>],
    known: &HashMap<String, &FileRecord>,
    previous_dir: Option<&Path>,
) -> io::Result<Vec<FileRecord>> {
    let mut records = Vec::with_capacity(files.len());
    for ((file, rel), hash) in files.iter().zip(stored).zip(hashes) {
        let dest = tmp.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // Unchanged since the previous backup: link to its copy instead of copying again.
        let previous = known.get(&portable::key(rel)).filter(|p| hash.as_ref() == Some(&p.hash));
        let linked = match (previous, previous_dir) {
            (Some(p), Some(prev_dir)) => fs::hard_link(prev_dir.join(p.stored_rel()), &dest).is_ok(),
            _ => false,
        };
        let hash = if linked {
            hash.clone().unwrap_or_default()
        } else {
            match fsutil::copy_hashed(&file.path, &dest) {
                Ok(hash) => hash,
                // The file was deleted after it was listed: leave it out.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let _ = fs::remove_file(&dest);
                    continue;
                }
                Err(e) => return Err(e),
            }
        };
        records.push(FileRecord {
            path: file.path.to_string_lossy().into_owned(),
            stored: Some(rel.to_string_lossy().replace('\\', "/")),
            size: file.size,
            mtime: file.mtime,
            hash,
        });
    }
    Ok(records)
}

fn prune(dir: &Path, keep: u32, protect: Option<&str>) {
    let keep = keep.max(1) as usize;
    let ids = backup_ids(dir);
    // A damaged save must never push out the last backup that was fine.
    let last_good = ids.iter().find(|id| read_meta(&dir.join(id)).is_ok_and(|m| m.suspect.is_none())).cloned();
    for id in ids.into_iter().skip(keep) {
        if Some(id.as_str()) != protect && Some(&id) != last_good.as_ref() {
            let _ = fs::remove_dir_all(dir.join(id));
        }
    }
    // Leftovers from an interrupted backup.
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().ends_with(".tmp") {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// What restoring a backup would overwrite.
pub struct Preview {
    pub files: u32,
    /// Files on disk that changed after this backup was made.
    pub newer: u32,
}

pub fn preview(root: &Path, game: &Game, id: &str) -> io::Result<Preview> {
    let loc = Locations::new(&UserFolders::detect(), game);
    let meta = read_meta(&game_dir(root, game).join(id))?;
    let mut newer = 0;
    for file in &meta.files {
        let target = match &file.stored {
            Some(stored) => loc.resolve(Path::new(stored)).ok_or_else(|| io::Error::other("pc_folder_missing"))?,
            None => PathBuf::from(&file.path),
        };
        if fs::metadata(&target).is_ok_and(|m| fsutil::mtime_nanos(&m) > file.mtime) {
            newer += 1;
        }
    }
    Ok(Preview { files: meta.files.len() as u32, newer })
}

/// Puts a backup's files back. The current saves are backed up first, so a
/// restore can always be undone.
pub fn restore(root: &Path, game: &Game, id: &str, keep: u32) -> io::Result<()> {
    let loc = Locations::new(&UserFolders::detect(), game);
    restore_in(root, game, id, keep, &loc)
}

fn restore_in(root: &Path, game: &Game, id: &str, keep: u32, loc: &Locations) -> io::Result<()> {
    let dir = game_dir(root, game).join(id);
    let meta = read_meta(&dir)?;

    // Work out every destination first, so a problem leaves the saves untouched.
    let mut targets = Vec::with_capacity(meta.files.len());
    for file in &meta.files {
        let target = match &file.stored {
            Some(stored) => loc.resolve(Path::new(stored)).ok_or_else(|| io::Error::other("pc_folder_missing"))?,
            None => PathBuf::from(&file.path),
        };
        targets.push(target);
    }

    backup_in(root, game, keep, "pre-restore", Some(id), loc)?;
    for (file, target) in meta.files.iter().zip(&targets) {
        let source = dir.join(file.stored_rel());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut tmp_name = target.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".savepoint-tmp");
        let tmp = target.with_file_name(tmp_name);
        fs::copy(&source, &tmp)?;
        fs::rename(&tmp, target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("savepoint-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn game(saves: &Path) -> Game {
        Game {
            name: "Test: Game".into(),
            custom: true,
            patterns: vec![glob::Pattern::escape(&saves.to_string_lossy())],
            ..Default::default()
        }
    }

    fn created(outcome: Outcome) -> Backup {
        match outcome {
            Outcome::Created(b) => b,
            other => panic!("expected a new backup, got {other:?}"),
        }
    }

    #[test]
    fn backup_skip_and_restore() {
        let tmp = TempDir::new("cycle");
        let saves = tmp.0.join("saves");
        let root = tmp.0.join("backups");
        fs::create_dir_all(saves.join("sub")).unwrap();
        fs::write(saves.join("a.sav"), "one").unwrap();
        fs::write(saves.join("sub").join("b.sav"), "bee").unwrap();
        let game = game(&saves);

        let first = created(backup(&root, &game, 10, "manual", None).unwrap());
        assert_eq!(first.meta.files.len(), 2);
        assert!(game_dir(&root, &game).ends_with("Test_ Game"));

        // Same content, newer mtime: still nothing to back up.
        fs::write(saves.join("a.sav"), "one").unwrap();
        assert!(matches!(backup(&root, &game, 10, "auto", None).unwrap(), Outcome::Unchanged));

        fs::write(saves.join("a.sav"), "two").unwrap();
        let second = created(backup(&root, &game, 10, "auto", None).unwrap());
        assert_eq!(list(&root, &game).len(), 2);

        restore(&root, &game, &first.id, 10).unwrap();
        assert_eq!(fs::read_to_string(saves.join("a.sav")).unwrap(), "one");
        assert_eq!(fs::read_to_string(saves.join("sub").join("b.sav")).unwrap(), "bee");
        // The current state ("two") was already the latest backup, so no extra copy.
        assert_eq!(list(&root, &game)[0].id, second.id);
    }

    #[test]
    fn prune_keeps_newest_and_protected() {
        let tmp = TempDir::new("prune");
        let saves = tmp.0.join("saves");
        let root = tmp.0.join("backups");
        fs::create_dir_all(&saves).unwrap();
        let game = game(&saves);
        let mut ids = Vec::new();
        for i in 0..4 {
            fs::write(saves.join("a.sav"), format!("v{i}")).unwrap();
            ids.push(created(backup(&root, &game, 2, "auto", Some("none")).unwrap()).id);
        }
        let kept: Vec<String> = list(&root, &game).into_iter().map(|b| b.id).collect();
        assert_eq!(kept, vec![ids[3].clone(), ids[2].clone()]);
    }

    #[test]
    fn flags_emptied_and_shrunken_saves_and_keeps_the_last_good_backup() {
        let tmp = TempDir::new("suspect");
        let saves = tmp.0.join("saves");
        let root = tmp.0.join("backups");
        fs::create_dir_all(&saves).unwrap();
        let game = game(&saves);
        let big = "a".repeat(10_000);

        fs::write(saves.join("a.sav"), &big).unwrap();
        fs::write(saves.join("b.sav"), &big).unwrap();
        let good = created(backup(&root, &game, 1, "auto", None).unwrap());
        assert_eq!(good.meta.suspect, None);

        // A normal change is not suspicious.
        fs::write(saves.join("a.sav"), "a".repeat(9_000)).unwrap();
        let normal = created(backup(&root, &game, 3, "auto", None).unwrap());
        assert_eq!(normal.meta.suspect, None);

        // One save file emptied by a crash.
        fs::write(saves.join("a.sav"), "").unwrap();
        let emptied = created(backup(&root, &game, 1, "auto", None).unwrap());
        assert_eq!(emptied.meta.suspect.as_deref(), Some("empty_file"));
        // keep = 1, yet the newest backup that was fine is still there.
        let ids: Vec<String> = list(&root, &game).into_iter().map(|b| b.id).collect();
        assert!(ids.contains(&emptied.id) && ids.contains(&normal.id), "{ids:?}");
        assert!(!ids.contains(&good.id));

        // Everything shrinking to almost nothing.
        fs::write(saves.join("a.sav"), &big).unwrap();
        let recovered = created(backup(&root, &game, 5, "auto", None).unwrap());
        assert_eq!(recovered.meta.suspect, None);
        fs::write(saves.join("a.sav"), "x").unwrap();
        fs::write(saves.join("b.sav"), "y").unwrap();
        let shrunk = created(backup(&root, &game, 5, "auto", None).unwrap());
        assert_eq!(shrunk.meta.suspect.as_deref(), Some("much_smaller"));
    }

    #[test]
    fn skips_saves_that_vanish_and_refuses_absurd_sizes() {
        let tmp = TempDir::new("vanish");
        let real = tmp.0.join("real.sav");
        fs::write(&real, "data").unwrap();
        let file = |path: &Path, size| SaveFile { path: path.to_path_buf(), size, mtime: 0 };
        let files = [file(&tmp.0.join("gone.sav"), 4), file(&real, 4)];
        let stored = [PathBuf::from("drive-X/gone.sav"), PathBuf::from("drive-X/real.sav")];
        let records = write_files(&tmp.0.join("out"), &files, &stored, &[None, None], &HashMap::new(), None).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].path.ends_with("real.sav"));

        assert!(check_size(&[file(&real, MAX_BACKUP_BYTES)]).is_ok());
        let err = check_size(&[file(&real, MAX_BACKUP_BYTES), file(&real, 1)]).unwrap_err();
        assert_eq!(err.to_string(), "too_large");
    }

    #[test]
    fn no_files_is_reported() {
        let tmp = TempDir::new("empty");
        let game = game(&tmp.0.join("missing"));
        assert!(matches!(backup(&tmp.0, &game, 10, "auto", None).unwrap(), Outcome::NoFiles));
    }

    fn folders(home: &Path) -> UserFolders {
        let h = home.to_string_lossy().into_owned();
        UserFolders {
            home: h.clone(),
            app_data: format!(r"{h}\AppData\Roaming"),
            local_app_data: format!(r"{h}\AppData\Local"),
            local_app_data_low: format!(r"{h}\AppData\LocalLow"),
            documents: format!(r"{h}\Documents"),
            public: format!(r"{h}\Public"),
            program_data: format!(r"{h}\ProgramData"),
            win_dir: String::new(),
            user_name: String::new(),
        }
    }

    #[test]
    fn restores_on_another_pc_with_different_user_and_install_folder() {
        let tmp = TempDir::new("move");
        let root = tmp.0.join("backups");

        // PC 1: saves in Documents and next to the game.
        let (home1, install1) = (tmp.0.join("pc1-ayse"), tmp.0.join("pc1-lib").join("Foo"));
        let docs1 = home1.join("Documents").join("Foo");
        fs::create_dir_all(&docs1).unwrap();
        fs::create_dir_all(install1.join("saves")).unwrap();
        fs::write(docs1.join("slot1.sav"), "documents save").unwrap();
        fs::write(install1.join("saves").join("slot2.sav"), "install save").unwrap();
        let on_pc1 = Game {
            name: "Foo".into(),
            patterns: vec![
                glob::Pattern::escape(&docs1.to_string_lossy()),
                glob::Pattern::escape(&install1.join("saves").to_string_lossy()),
            ],
            install_dirs: vec![install1.to_string_lossy().into_owned()],
            ..Default::default()
        };
        let loc1 = Locations::new(&folders(&home1), &on_pc1);
        let first = created(backup_in(&root, &on_pc1, 10, "manual", None, &loc1).unwrap());
        let stored: Vec<&str> = first.meta.files.iter().filter_map(|f| f.stored.as_deref()).collect();
        assert!(stored.contains(&"documents/Foo/slot1.sav"), "{stored:?}");
        assert!(stored.contains(&"install/saves/slot2.sav"), "{stored:?}");

        // PC 2: nothing installed with saves yet, different user and game folder.
        let (home2, install2) = (tmp.0.join("pc2-mehmet"), tmp.0.join("pc2-games").join("Foo"));
        fs::create_dir_all(&install2).unwrap();
        let on_pc2 = Game {
            name: "Foo".into(),
            install_dirs: vec![install2.to_string_lossy().into_owned()],
            ..Default::default()
        };
        let loc2 = Locations::new(&folders(&home2), &on_pc2);
        restore_in(&root, &on_pc2, &first.id, 10, &loc2).unwrap();
        assert_eq!(
            fs::read_to_string(home2.join("Documents").join("Foo").join("slot1.sav")).unwrap(),
            "documents save"
        );
        assert_eq!(fs::read_to_string(install2.join("saves").join("slot2.sav")).unwrap(), "install save");

        // A game that is not installed on PC 3 cannot be restored, and nothing is written.
        let home3 = tmp.0.join("pc3");
        let on_pc3 = Game { name: "Foo".into(), ..Default::default() };
        let loc3 = Locations::new(&folders(&home3), &on_pc3);
        assert!(restore_in(&root, &on_pc3, &first.id, 10, &loc3).is_err());
        assert!(!home3.exists());
    }

    #[test]
    fn finds_games_that_only_exist_in_the_backup_folder() {
        let tmp = TempDir::new("discover");
        let saves = tmp.0.join("saves");
        let root = tmp.0.join("backups");
        fs::create_dir_all(&saves).unwrap();
        fs::write(saves.join("a.sav"), "x").unwrap();
        created(backup(&root, &game(&saves), 10, "manual", None).unwrap());
        fs::create_dir_all(root.join("not a game")).unwrap();
        assert_eq!(discover(&root), vec!["Test: Game".to_string()]);
    }
}
