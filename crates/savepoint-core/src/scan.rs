//! Matches manifest entries against this machine to find which games have
//! saves here (or are installed through Steam).
//!
//! Tens of thousands of paths get checked, so directory listings are cached
//! and a glob only runs once its literal prefix is known to exist.

use crate::game::{Game, has_wildcard};
use crate::manifest::Entry;
use crate::paths::UserFolders;
use crate::{fsutil, installs};
use glob::Pattern;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn games_file(data_dir: &Path) -> PathBuf {
    data_dir.join("games.json")
}

pub fn load_games(data_dir: &Path) -> Option<Vec<Game>> {
    let bytes = fs::read(games_file(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_games(data_dir: &Path, games: &[Game]) -> io::Result<()> {
    fsutil::write_atomic(&games_file(data_dir), &serde_json::to_vec(games)?)
}

pub fn detect(
    entries: &[Entry],
    folders: &UserFolders,
    steam_libs: &[PathBuf],
    others: &[installs::Install],
) -> Vec<Game> {
    let mut scanner = Scanner {
        fs: DirCache::default(),
        vars: placeholder_values(folders),
        steam_libs: steam_libs.iter().map(|l| slash(&l.to_string_lossy())).collect(),
        by_name: HashMap::new(),
        by_gog_id: HashMap::new(),
        emulator_roots: Vec::new(),
    };
    for (i, install) in others.iter().enumerate() {
        let name = installs::normalize_name(&install.name);
        if !installs::is_generic_folder(&name) {
            scanner.by_name.entry(name).or_default().push(i);
        }
        let folder = installs::normalize_name(install.path.rsplit('\\').next().unwrap_or(""));
        if !installs::is_generic_folder(&folder) {
            scanner.by_name.entry(folder).or_default().push(i);
        }
        if let Some(id) = install.gog_id {
            scanner.by_gog_id.entry(id).or_default().push(i);
        }
    }
    scanner.emulator_roots = scanner.existing_emulator_roots(folders);
    let mut games: Vec<Game> = entries.iter().filter_map(|e| scanner.detect(e, others)).collect();
    games.sort_by_key(|g| g.name.to_lowercase());
    games
}

/// Where cracked Steam games keep the saves their emulator redirects, one folder per app id.
const EMULATOR_ROOTS: &[(&str, &str)] = &[
    ("public", "Documents/Steam/CODEX"),
    ("public", "Documents/Steam/RUNE"),
    ("public", "Documents/OnlineFix"),
    ("public", "Documents/EMPRESS"),
    ("app_data", "Goldberg SteamEmu Saves"),
    ("app_data", "GSE Saves"),
    ("app_data", "CPY_SAVES"),
    ("app_data", "EMPRESS"),
    ("app_data", "SmartSteamEmu"),
];

struct Scanner {
    fs: DirCache,
    vars: Vec<(&'static str, String)>,
    steam_libs: Vec<String>,
    /// Install folders by normalized folder or launcher name.
    by_name: HashMap<String, Vec<usize>>,
    by_gog_id: HashMap<u64, Vec<usize>>,
    emulator_roots: Vec<String>,
}

/// A place the game is installed. `base` is the game's own folder, `root` the
/// folder that holds it (a Steam library, or the parent folder elsewhere).
struct Install {
    root: String,
    dir: String,
    base: String,
    trusted: bool,
}

impl Scanner {
    fn existing_emulator_roots(&mut self, f: &UserFolders) -> Vec<String> {
        let mut roots = Vec::new();
        for (base, rel) in EMULATOR_ROOTS {
            let base = if *base == "public" { &f.public } else { &f.app_data };
            let root = format!("{}/{rel}", slash(base));
            if self.fs.exists(&root) {
                roots.push(root);
            }
        }
        roots
    }

    fn detect(&mut self, entry: &Entry, others: &[installs::Install]) -> Option<Game> {
        let libs = self.steam_libs.clone();
        let mut found: Vec<Install> = Vec::new();
        for lib in &libs {
            for dir in &entry.install_dirs {
                let base = format!("{lib}/steamapps/common/{dir}");
                if self.fs.exists(&base) {
                    found.push(Install { root: lib.clone(), dir: dir.clone(), base, trusted: true });
                }
            }
        }
        for i in self.matching_installs(entry) {
            let path = slash(&others[i].path);
            let Some((root, dir)) = path.rsplit_once('/') else { continue };
            if found.iter().any(|f| f.base.eq_ignore_ascii_case(&path)) || !self.fs.exists(&path) {
                continue;
            }
            found.push(Install {
                root: root.to_string(),
                dir: dir.to_string(),
                base: path,
                trusted: others[i].trusted,
            });
        }

        let mut patterns = Vec::new();
        for file in &entry.files {
            for pattern in self.expand(file, &found, &libs) {
                if !patterns.contains(&pattern) {
                    patterns.push(pattern);
                }
            }
        }
        if let Some(id) = entry.steam_id {
            for root in &self.emulator_roots {
                patterns.push(Pattern::escape(&format!("{root}/{id}")));
            }
        }

        let has_saves = patterns.iter().any(|p| self.any_file(p));
        // Without saves, only a launcher-confirmed install is enough to list a game.
        if !has_saves && !found.iter().any(|i| i.trusted) {
            return None;
        }
        Some(Game {
            name: entry.name.clone(),
            custom: false,
            patterns,
            install_dirs: found.iter().map(|i| i.base.replace('/', "\\")).collect(),
            install_names: entry.install_dirs.clone(),
            exes: Vec::new(),
            steam_id: entry.steam_id,
        })
    }

    /// Launcher or folder installs that belong to this game: same GOG id, or the
    /// same name once punctuation is ignored ("Cyberpunk.2077" = "Cyberpunk 2077").
    fn matching_installs(&self, entry: &Entry) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        if let Some(list) = entry.gog_id.and_then(|id| self.by_gog_id.get(&id)) {
            out.extend(list);
        }
        let mut names: Vec<String> = entry.install_dirs.iter().map(|d| installs::normalize_name(d)).collect();
        names.push(installs::normalize_name(&entry.name));
        for name in names {
            if let Some(list) = self.by_name.get(&name) {
                out.extend(list);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Replaces placeholders, yielding one pattern per possible install location.
    fn expand(&self, raw: &str, installs: &[Install], libs: &[String]) -> Vec<String> {
        let mut path = raw.to_string();
        for (key, value) in &self.vars {
            if path.contains(key) {
                path = path.replace(key, value);
            }
        }
        path = path.replace("<storeUserId>", "*");
        if path.contains("<storeGameId>") {
            return Vec::new();
        }

        let mut out = Vec::new();
        if path.contains("<base>") || path.contains("<game>") {
            for install in installs {
                let root = Pattern::escape(&install.root);
                let dir = Pattern::escape(&install.dir);
                let base = Pattern::escape(&install.base);
                out.push(path.replace("<base>", &base).replace("<game>", &dir).replace("<root>", &root));
            }
        } else if path.contains("<root>") {
            for lib in libs {
                out.push(path.replace("<root>", &Pattern::escape(lib)));
            }
        } else {
            out.push(path);
        }
        out.retain(|p| !p.contains('<') && is_absolute(p));
        out
    }

    fn any_file(&mut self, pattern: &str) -> bool {
        let literal_end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
        let prefix = &pattern[..literal_end];
        let dir =
            if has_wildcard(pattern) { prefix.rsplit_once('/').map(|(d, _)| d).unwrap_or(prefix) } else { prefix };
        if !self.fs.exists(dir) {
            return false;
        }
        let game = Game { patterns: vec![pattern.to_string()], ..Default::default() };
        !game.collect_files().is_empty()
    }
}

fn placeholder_values(f: &UserFolders) -> Vec<(&'static str, String)> {
    let v = |s: &str| Pattern::escape(&slash(s));
    vec![
        ("<home>", v(&f.home)),
        ("<winAppData>", v(&f.app_data)),
        ("<winLocalAppDataLow>", v(&f.local_app_data_low)),
        ("<winLocalAppData>", v(&f.local_app_data)),
        ("<winDocuments>", v(&f.documents)),
        ("<winPublic>", v(&f.public)),
        ("<winProgramData>", v(&f.program_data)),
        ("<winDir>", v(&f.win_dir)),
        ("<osUserName>", Pattern::escape(&f.user_name)),
    ]
}

fn slash(s: &str) -> String {
    s.replace('\\', "/").trim_end_matches('/').to_string()
}

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'/'
}

/// Case-insensitive existence checks backed by one `read_dir` per folder.
#[derive(Default)]
struct DirCache {
    dirs: HashMap<String, Option<HashSet<String>>>,
}

impl DirCache {
    fn exists(&mut self, path: &str) -> bool {
        let path = path.trim_end_matches('/');
        let Some((parent, name)) = path.rsplit_once('/') else {
            // A bare drive such as "C:".
            return Path::new(&format!("{path}/")).exists();
        };
        if !self.exists(parent) {
            return false;
        }
        let key = parent.to_lowercase();
        let listing = self.dirs.entry(key).or_insert_with(|| {
            let dir = if parent.ends_with(':') { format!("{parent}/") } else { parent.to_string() };
            fs::read_dir(dir)
                .ok()
                .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
        });
        match listing {
            Some(names) => names.contains(&name.to_lowercase()),
            None => Path::new(path).exists(),
        }
    }
}
