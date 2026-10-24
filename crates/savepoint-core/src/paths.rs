//! Where Savepoint keeps its own files, and the user folders that game save
//! paths are relative to.

use crate::win;
use std::env;
use std::path::PathBuf;

/// `%LOCALAPPDATA%\Savepoint`: config, cached manifest, scan results, log.
pub fn data_dir() -> PathBuf {
    let base = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir);
    let dir = base.join("Savepoint");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Folders that the manifest's `<placeholders>` stand for on this machine.
#[derive(Debug, Clone)]
pub struct UserFolders {
    pub home: String,
    pub app_data: String,
    pub local_app_data: String,
    pub local_app_data_low: String,
    pub documents: String,
    pub public: String,
    pub program_data: String,
    pub win_dir: String,
    pub user_name: String,
}

impl UserFolders {
    pub fn detect() -> Self {
        let var = |name: &str| env::var(name).unwrap_or_default();
        let home = var("USERPROFILE");
        let documents = win::known_folder(&win::FOLDERID_Documents)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{home}\\Documents"));
        let local_app_data_low = win::known_folder(&win::FOLDERID_LocalAppDataLow)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{home}\\AppData\\LocalLow"));
        UserFolders {
            app_data: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
            local_app_data_low,
            documents,
            public: var("PUBLIC"),
            program_data: var("PROGRAMDATA"),
            win_dir: var("WINDIR"),
            user_name: var("USERNAME"),
            home,
        }
    }
}

/// Steam install folder plus every library folder listed in `libraryfolders.vdf`.
pub fn steam_libraries() -> Vec<PathBuf> {
    let root = win::reg_get_string(win::HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath")
        .or_else(|| win::reg_get_string(win::HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"));
    let Some(root) = root else { return Vec::new() };
    let root = PathBuf::from(root.replace('/', "\\"));
    let mut libs = vec![root.clone()];
    let vdf = std::fs::read_to_string(root.join(r"steamapps\libraryfolders.vdf")).unwrap_or_default();
    for line in vdf.lines() {
        let line = line.trim();
        if !line.starts_with("\"path\"") {
            continue;
        }
        let value = line["\"path\"".len()..].trim().trim_matches('"').replace("\\\\", "\\");
        let lib = PathBuf::from(value);
        if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&lib.to_string_lossy())) {
            libs.push(lib);
        }
    }
    libs.retain(|l| l.is_dir());
    libs
}
