//! Backups remember where a file lived relative to a well-known folder, not as
//! an absolute path, so they can be restored on another PC where the user name,
//! the Steam library or the drive letter is different:
//!
//! ```text
//! C:\Users\me\Documents\Game\save.dat      ->  documents/Game/save.dat
//! D:\SteamLibrary\...\Game\saves\a.sav     ->  install/saves/a.sav
//! E:\Stuff\save.dat                        ->  drive-E/Stuff/save.dat
//! ```

use crate::game::Game;
use crate::paths::UserFolders;
use std::path::{Component, Path, PathBuf, Prefix};

pub struct Locations {
    /// (name, folder) for the user folders, e.g. ("documents", "C:\Users\me\Documents").
    roots: Vec<(String, String)>,
    /// Where the game is installed on this PC, if known.
    installs: Vec<String>,
}

impl Locations {
    pub fn new(folders: &UserFolders, game: &Game) -> Self {
        let roots = [
            ("documents", &folders.documents),
            ("localappdatalow", &folders.local_app_data_low),
            ("localappdata", &folders.local_app_data),
            ("appdata", &folders.app_data),
            ("programdata", &folders.program_data),
            ("public", &folders.public),
            ("home", &folders.home),
        ]
        .into_iter()
        .filter(|(_, path)| !path.is_empty())
        .map(|(name, path)| (name.to_string(), clean(path)))
        .collect();
        Locations { roots, installs: game.install_dirs.iter().map(|d| clean(d)).collect() }
    }

    /// The well-known folder a path is inside, and where the rest of the path starts.
    fn split(&self, full: &str) -> Option<(&str, usize)> {
        let candidates = self
            .roots
            .iter()
            .map(|(n, p)| (n.as_str(), p.as_str()))
            .chain(self.installs.iter().map(|p| ("install", p.as_str())));
        let mut best: Option<(&str, usize)> = None;
        for (name, prefix) in candidates {
            if is_inside(full, prefix) && best.is_none_or(|(_, len)| prefix.len() > len) {
                best = Some((name, prefix.len()));
            }
        }
        best
    }

    /// The path a file is stored under inside a backup.
    pub fn to_stored(&self, original: &Path) -> PathBuf {
        let full = clean(&original.to_string_lossy());
        match self.split(&full) {
            Some((name, len)) => PathBuf::from(name).join(full[len..].trim_start_matches('\\')),
            None => drive_stored(original),
        }
    }

    /// Where a stored path belongs on this PC; `None` if its base folder is unknown here.
    pub fn resolve(&self, stored: &Path) -> Option<PathBuf> {
        let mut parts = stored.components();
        let Some(Component::Normal(first)) = parts.next() else { return None };
        let first = first.to_string_lossy();
        let rest: PathBuf = parts.collect();
        let base = if first == "install" {
            let dir = self.installs.iter().find(|d| Path::new(d).is_dir()).or(self.installs.first())?;
            PathBuf::from(dir)
        } else if let Some(letter) = first.strip_prefix("drive-") {
            PathBuf::from(format!("{letter}:\\"))
        } else {
            PathBuf::from(&self.roots.iter().find(|(name, _)| *name == *first)?.1)
        };
        Some(base.join(rest))
    }
}

/// `C:\Users\x` -> `drive-C/Users/x`. Also the layout of older backups.
pub fn drive_stored(original: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in original.components() {
        match component {
            Component::Prefix(p) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => out.push(format!("drive-{}", d as char)),
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    out.push(format!("unc-{}-{}", server.to_string_lossy(), share.to_string_lossy()))
                }
                _ => out.push("other"),
            },
            Component::Normal(part) => out.push(part),
            _ => {}
        }
    }
    out
}

/// Case-insensitive comparison key for a stored path.
pub fn key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn clean(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_string()
}

/// `path` is `folder` itself or something below it. Compares bytes, so folder
/// names with non-ASCII letters (common in user names) are safe.
fn is_inside(path: &str, folder: &str) -> bool {
    let (p, f) = (path.as_bytes(), folder.as_bytes());
    p.len() >= f.len() && p[..f.len()].eq_ignore_ascii_case(f) && (p.len() == f.len() || p[f.len()] == b'\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders(home: &str) -> UserFolders {
        UserFolders {
            home: home.to_string(),
            app_data: format!(r"{home}\AppData\Roaming"),
            local_app_data: format!(r"{home}\AppData\Local"),
            local_app_data_low: format!(r"{home}\AppData\LocalLow"),
            documents: format!(r"{home}\Documents"),
            public: r"C:\Users\Public".to_string(),
            program_data: r"C:\ProgramData".to_string(),
            win_dir: r"C:\Windows".to_string(),
            user_name: "x".to_string(),
        }
    }

    fn game(install: &[&str]) -> Game {
        Game { install_dirs: install.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn stores_relative_to_known_folders() {
        let loc = Locations::new(&folders(r"C:\Users\ayşe"), &game(&[r"D:\Lib\steamapps\common\Foo"]));
        let stored = |p: &str| loc.to_stored(Path::new(p)).to_string_lossy().replace('\\', "/");
        assert_eq!(stored(r"C:\Users\ayşe\Documents\Foo\a.sav"), "documents/Foo/a.sav");
        assert_eq!(stored(r"C:\Users\AYşE\AppData\Roaming\Foo\a.sav"), "appdata/Foo/a.sav");
        assert_eq!(stored(r"C:\Users\ayşe\Saved Games\Foo\a.sav"), "home/Saved Games/Foo/a.sav");
        assert_eq!(stored(r"D:\Lib\steamapps\common\Foo\saves\a.sav"), "install/saves/a.sav");
        assert_eq!(stored(r"E:\Stuff\a.sav"), "drive-E/Stuff/a.sav");
        // A folder merely sharing a prefix is not inside it.
        assert_eq!(stored(r"C:\Users\ayşe\DocumentsOld\a.sav"), "home/DocumentsOld/a.sav");
    }

    #[test]
    fn resolves_on_another_pc() {
        let old = Locations::new(&folders(r"C:\Users\ayse"), &game(&[r"D:\Lib\steamapps\common\Foo"]));
        let new = Locations::new(&folders(r"C:\Users\mehmet"), &game(&[r"F:\Games\Foo"]));
        for (original, expected) in [
            (r"C:\Users\ayse\Documents\Foo\a.sav", r"C:\Users\mehmet\Documents\Foo\a.sav"),
            (r"C:\Users\ayse\AppData\Local\Foo\a.sav", r"C:\Users\mehmet\AppData\Local\Foo\a.sav"),
            (r"D:\Lib\steamapps\common\Foo\saves\a.sav", r"F:\Games\Foo\saves\a.sav"),
            (r"E:\Stuff\a.sav", r"E:\Stuff\a.sav"),
        ] {
            let stored = old.to_stored(Path::new(original));
            assert_eq!(new.resolve(&stored), Some(PathBuf::from(expected)), "{original}");
        }
    }

    #[test]
    fn unknown_install_does_not_resolve() {
        let old = Locations::new(&folders(r"C:\Users\a"), &game(&[r"D:\Foo"]));
        let stored = old.to_stored(Path::new(r"D:\Foo\a.sav"));
        let new = Locations::new(&folders(r"C:\Users\b"), &game(&[]));
        assert_eq!(new.resolve(&stored), None);
    }
}
