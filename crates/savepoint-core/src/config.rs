use crate::game::Game;
use crate::{fsutil, paths, win};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub backup_dir: String,
    /// Backups kept per game; older ones are deleted.
    pub keep: u32,
    /// Show the notification-area icon. The service runs either way.
    pub show_tray: bool,
    /// "auto" follows Windows; otherwise a language code such as "en".
    pub lang: String,
    /// Detected games the user removed from the list.
    pub hidden: Vec<String>,
    pub custom: Vec<CustomGame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomGame {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub exe: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let documents = win::known_folder(&win::FOLDERID_Documents)
            .unwrap_or_else(|| PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()));
        Config {
            backup_dir: documents.join("Savepoint Backups").to_string_lossy().into_owned(),
            keep: 10,
            show_tray: true,
            lang: "auto".to_string(),
            hidden: Vec::new(),
            custom: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        paths::data_dir().join("config.json")
    }

    /// Returns the config and whether it already existed.
    pub fn load() -> (Config, bool) {
        match std::fs::read(Self::path()) {
            Ok(bytes) => (serde_json::from_slice(&bytes).unwrap_or_default(), true),
            Err(_) => (Config::default(), false),
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        fsutil::write_atomic(&Self::path(), &bytes)
    }
}

impl CustomGame {
    pub fn to_game(&self) -> Game {
        let exe = self.exe.clone().filter(|e| !e.is_empty());
        let install_dirs = exe
            .as_deref()
            .and_then(|e| std::path::Path::new(e).parent())
            .map(|p| vec![p.to_string_lossy().into_owned()])
            .unwrap_or_default();
        Game {
            name: self.name.clone(),
            custom: true,
            patterns: vec![glob::Pattern::escape(&self.path)],
            install_dirs,
            install_names: Vec::new(),
            exes: exe.into_iter().collect(),
            steam_id: None,
        }
    }
}
