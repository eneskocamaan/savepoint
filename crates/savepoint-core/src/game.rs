use crate::fsutil;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// A game whose saves Savepoint knows how to find on this machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Game {
    pub name: String,
    #[serde(default)]
    pub custom: bool,
    /// Absolute glob patterns; a match that is a folder means "everything inside".
    pub patterns: Vec<String>,
    /// Install folders, used to tell that one of the game's processes is running.
    #[serde(default)]
    pub install_dirs: Vec<String>,
    /// Install folder names from the manifest, for games installed outside Steam.
    #[serde(default)]
    pub install_names: Vec<String>,
    /// Exact executable paths, for games added by hand.
    #[serde(default)]
    pub exes: Vec<String>,
    /// Steam app id from the manifest; also recognizes cracked copies of the game.
    #[serde(default)]
    pub steam_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

const GLOB_OPTIONS: glob::MatchOptions =
    glob::MatchOptions { case_sensitive: false, require_literal_separator: true, require_literal_leading_dot: false };

pub fn has_wildcard(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

impl Game {
    pub fn folder_name(&self) -> String {
        fsutil::safe_name(&self.name)
    }

    /// Every save file currently on disk, sorted by path.
    pub fn collect_files(&self) -> Vec<SaveFile> {
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for pattern in &self.patterns {
            if has_wildcard(pattern) {
                let Ok(paths) = glob::glob_with(pattern, GLOB_OPTIONS) else { continue };
                for path in paths.flatten() {
                    add_path(path, &mut seen, &mut files);
                }
            } else {
                add_path(PathBuf::from(pattern), &mut seen, &mut files);
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }

    /// Display-friendly save locations: each pattern cut at its first wildcard.
    pub fn locations(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for pattern in &self.patterns {
            let literal = match pattern.find(['*', '?', '[']) {
                Some(i) => pattern[..i].rsplit_once('/').map(|(dir, _)| dir).unwrap_or(&pattern[..i]),
                None => pattern.as_str(),
            };
            let shown = literal.replace('/', "\\");
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&shown)) {
                out.push(shown);
            }
        }
        // Show where saves actually are; fall back to every candidate when none exist yet.
        let existing: Vec<String> = out.iter().filter(|p| std::path::Path::new(p).exists()).cloned().collect();
        if existing.is_empty() { out } else { existing }
    }
}

fn add_path(path: PathBuf, seen: &mut HashSet<String>, out: &mut Vec<SaveFile>) {
    let Ok(meta) = fs::symlink_metadata(&path) else { return };
    if meta.is_dir() {
        let Ok(entries) = fs::read_dir(&path) else { return };
        for entry in entries.flatten() {
            add_path(entry.path(), seen, out);
        }
    } else if meta.is_file() {
        let key = path.to_string_lossy().to_lowercase().replace('/', "\\");
        if seen.insert(key) {
            let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
            out.push(SaveFile { path, size: meta.len(), mtime: fsutil::mtime_nanos(&meta) });
        }
    }
}
