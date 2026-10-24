//! Finds where games are installed outside Steam, and recognizes cracked
//! copies, which register with no launcher at all.
//!
//! - Epic Games: JSON files the launcher keeps for each install.
//! - GOG Galaxy: one registry key per game.
//! - "Installed programs" registry keys: installers of every kind, repacks and
//!   cracked copies included, record a name and an install folder there.
//! - Xbox app: `X:\XboxGames\<Game>` on any fixed drive.
//! - Everything else: the immediate subfolders of the usual game folders
//!   (`Games`, `EA Games`, `Ubisoft Game Launcher\games`, ...) on each drive.
//! - Cracked games: the exe's folder usually holds a `steam_appid.txt` or an
//!   emulator `.ini` that names the real Steam app id.

use crate::paths::UserFolders;
use crate::win;
use std::fs;
use std::path::Path;

/// A game folder found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// Name shown by the launcher, or the folder name.
    pub name: String,
    /// Absolute path, backslash-separated.
    pub path: String,
    pub gog_id: Option<u64>,
    /// From a launcher that only lists games (Epic, GOG, Xbox). Registry entries
    /// and folder names also match ordinary apps, so they are trusted less.
    pub trusted: bool,
}

/// Folders under each drive whose subfolders are usually one game each.
const FOLDER_ROOTS: &[&str] = &[
    r"Games",
    r"Game",
    r"Oyunlar",
    r"GOG Games",
    r"Epic Games",
    r"Amazon Games\Library",
    r"Program Files\EA Games",
    r"Program Files (x86)\Origin Games",
    r"Program Files\Ubisoft\Ubisoft Game Launcher\games",
    r"Program Files (x86)\Ubisoft\Ubisoft Game Launcher\games",
];

pub fn discover(folders: &UserFolders) -> Vec<Install> {
    let mut out = Vec::new();
    epic(&folders.program_data, &mut out);
    gog(&mut out);
    uninstall_entries(&mut out);
    for drive in win::fixed_drives() {
        xbox(&drive, &mut out);
        for root in FOLDER_ROOTS {
            subfolders(&format!("{drive}{root}"), &mut out);
        }
    }
    out
}

fn subfolders(root: &str, out: &mut Vec<Install>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            out.push(Install {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                gog_id: None,
                trusted: false,
            });
        }
    }
}

fn epic(program_data: &str, out: &mut Vec<Install>) {
    let dir = Path::new(program_data).join(r"Epic\EpicGamesLauncher\Data\Manifests");
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|e| e.eq_ignore_ascii_case("item"))
            && let Ok(text) = fs::read_to_string(entry.path())
            && let Some(install) = parse_epic_item(&text)
        {
            out.push(install);
        }
    }
}

pub fn parse_epic_item(json: &str) -> Option<Install> {
    let v: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    if v["bIsIncompleteInstall"].as_bool() == Some(true) {
        return None;
    }
    let path = v["InstallLocation"].as_str()?.replace('/', "\\");
    let name = v["DisplayName"].as_str().or_else(|| v["AppName"].as_str())?;
    Path::new(&path).is_dir().then(|| Install { name: name.to_string(), path, gog_id: None, trusted: true })
}

fn gog(out: &mut Vec<Install>) {
    const KEY: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";
    for id in win::reg_subkeys(win::HKEY_LOCAL_MACHINE, KEY) {
        let key = format!(r"{KEY}\{id}");
        let Some(path) = win::reg_get_string(win::HKEY_LOCAL_MACHINE, &key, "path") else { continue };
        if !Path::new(&path).is_dir() {
            continue;
        }
        let name = win::reg_get_string(win::HKEY_LOCAL_MACHINE, &key, "gameName").unwrap_or_default();
        out.push(Install { name, path, gog_id: id.parse().ok(), trusted: true });
    }
}

/// Every installer that registers itself leaves a name and, usually, an install folder here.
fn uninstall_entries(out: &mut Vec<Install>) {
    const KEYS: &[&str] = &[
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    for root in [win::HKEY_LOCAL_MACHINE, win::HKEY_CURRENT_USER] {
        for base in KEYS {
            for id in win::reg_subkeys(root, base) {
                let key = format!(r"{base}\{id}");
                let Some(name) = win::reg_get_string(root, &key, "DisplayName") else { continue };
                let location = win::reg_get_string(root, &key, "InstallLocation")
                    .filter(|l| !l.trim().is_empty())
                    .or_else(|| win::reg_get_string(root, &key, "DisplayIcon").and_then(|i| folder_of_icon(&i)));
                let Some(path) = location.map(|l| l.trim().trim_matches('"').trim_end_matches('\\').to_string()) else {
                    continue;
                };
                if Path::new(&path).is_dir() {
                    out.push(Install { name, path, gog_id: None, trusted: false });
                }
            }
        }
    }
}

/// `DisplayIcon` is usually `"C:\Games\X\x.exe",0`; the exe's folder is the install folder.
fn folder_of_icon(icon: &str) -> Option<String> {
    let file = match icon.rsplit_once(',') {
        Some((file, index)) if index.trim().parse::<i32>().is_ok() => file,
        _ => icon,
    };
    let file = file.trim().trim_matches('"');
    Path::new(file).parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| p.len() > 3)
}

fn xbox(drive: &str, out: &mut Vec<Install>) {
    let Ok(entries) = fs::read_dir(format!("{drive}XboxGames")) else { return };
    for entry in entries.flatten() {
        let content = entry.path().join("Content");
        let path = if content.is_dir() { content } else { entry.path() };
        if path.is_dir() {
            out.push(Install {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                gog_id: None,
                trusted: true,
            });
        }
    }
}

/// Lowercase letters and digits only, so "Cyberpunk.2077" == "Cyberpunk 2077".
pub fn normalize_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Folder names that say nothing about which game they belong to.
pub fn is_generic_folder(normalized: &str) -> bool {
    const GENERIC: &[&str] = &[
        "bin",
        "bin32",
        "bin64",
        "binaries",
        "win64",
        "win32",
        "x64",
        "x86",
        "game",
        "games",
        "data",
        "common",
        "steamapps",
        "steamlibrary",
        "programfiles",
        "programfilesx86",
        "epicgames",
        "goggames",
        "goggalaxy",
        "xboxgames",
        "content",
        "windows",
        "launcher",
        "client",
        "oyunlar",
        "indirilenler",
        "downloads",
        "desktop",
        "retail",
        "release",
        "shipping",
        "windowsnoeditor",
        "app",
        "apps",
        "pc",
        "program",
        "setup",
        "install",
        "installer",
        "crack",
        "fix",
        "engine",
        "assets",
        "tools",
        "users",
        "library",
        "amazon",
        "origin",
    ];
    normalized.len() < 4 || GENERIC.contains(&normalized)
}

// ---- cracked games ----

/// Files that name the Steam app id a cracked game pretends to be.
const APPID_FILES: &[&str] = &[
    "steam_appid.txt",
    r"steam_settings\steam_appid.txt",
    "OnlineFix.ini",
    "steam_emu.ini",
    "ALI213.ini",
    "SmartSteamEmu.ini",
    "ColdClientLoader.ini",
    "cream_api.ini",
];

/// Looks for an app id file next to the exe, or up to three folders above it.
pub fn appid_near(exe_dir: &Path) -> Option<u64> {
    let mut dir = Some(exe_dir);
    for _ in 0..4 {
        let d = dir?;
        for name in APPID_FILES {
            if let Ok(text) = fs::read_to_string(d.join(name))
                && let Some(id) = parse_appid(name, &text)
            {
                return Some(id);
            }
        }
        dir = d.parent();
    }
    None
}

pub fn parse_appid(file: &str, text: &str) -> Option<u64> {
    if file.ends_with(".txt") {
        return text.trim().parse().ok().filter(|&id| id > 0);
    }
    // INI files: prefer the real id when a fake one is also present (OnlineFix).
    let mut fallback = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let Ok(id) = value.trim().parse::<u64>() else { continue };
        if id == 0 {
            continue;
        }
        match key.as_str() {
            "realappid" => return Some(id),
            "appid" if fallback.is_none() => fallback = Some(id),
            _ => {}
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_name("Cyberpunk.2077"), normalize_name("Cyberpunk 2077"));
        assert_eq!(normalize_name("The Witcher 3: Wild Hunt"), "thewitcher3wildhunt");
    }

    #[test]
    fn generic_folders_are_skipped() {
        assert!(is_generic_folder("bin"));
        assert!(is_generic_folder("games"));
        assert!(is_generic_folder("ab"));
        assert!(!is_generic_folder("cyberpunk2077"));
    }

    #[test]
    fn parses_appid_files() {
        assert_eq!(parse_appid("steam_appid.txt", "1091500\r\n"), Some(1091500));
        assert_eq!(parse_appid("steam_appid.txt", "nope"), None);
        assert_eq!(parse_appid("steam_emu.ini", "[Settings]\nAppId=730\nUserName=x\n"), Some(730));
        assert_eq!(parse_appid("OnlineFix.ini", "[Main]\nFakeAppId=480\nRealAppId = 1245620\n"), Some(1245620));
        assert_eq!(parse_appid("ALI213.ini", "[Settings]\nAppID=0\n"), None);
    }

    #[test]
    fn finds_install_folder_from_display_icon() {
        assert_eq!(folder_of_icon(r#""D:\Games\Foo Bar\foo.exe",0"#).as_deref(), Some(r"D:\Games\Foo Bar"));
        assert_eq!(folder_of_icon(r"D:\Games\Foo\foo.exe").as_deref(), Some(r"D:\Games\Foo"));
        assert_eq!(folder_of_icon(r"C:\setup.exe"), None);
    }

    #[test]
    fn parses_epic_manifest() {
        let dir = std::env::temp_dir();
        let json = format!(
            r#"{{"DisplayName":"Some Game","AppName":"abc","InstallLocation":"{}","bIsIncompleteInstall":false}}"#,
            dir.to_string_lossy().replace('\\', "\\\\")
        );
        let install = parse_epic_item(&json).unwrap();
        assert_eq!(install.name, "Some Game");
        assert!(parse_epic_item(&json.replace("false", "true")).is_none());
        assert!(parse_epic_item(r#"{"DisplayName":"X","InstallLocation":"Z:\\nope"}"#).is_none());
    }
}
