//! The Ludusavi Manifest: a community list of where thousands of games keep
//! their saves, mostly sourced from PCGamingWiki.
//!
//! The YAML is large (~17 MB), so it is parsed only when it changes and boiled
//! down to a compact `index.json` holding just what Windows needs.

use crate::fsutil;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const URL: &str = "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One game, reduced to what matters on Windows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub install_dirs: Vec<String>,
    #[serde(default)]
    pub steam_id: Option<u64>,
    #[serde(default)]
    pub gog_id: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawGame {
    files: HashMap<String, Option<RawFile>>,
    #[serde(rename = "installDir")]
    install_dir: HashMap<String, IgnoredAny>,
    steam: Option<RawSteam>,
    gog: Option<RawSteam>,
    alias: Option<IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFile {
    when: Vec<RawWhen>,
    tags: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawWhen {
    os: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSteam {
    id: Option<u64>,
}

struct Files {
    yaml: PathBuf,
    etag: PathBuf,
    index: PathBuf,
}

impl Files {
    fn new(data_dir: &Path) -> Self {
        Files {
            yaml: data_dir.join("manifest.yaml"),
            etag: data_dir.join("manifest.etag"),
            index: data_dir.join("index-v3.json"),
        }
    }
}

/// Downloads a fresh manifest if the cached one is older than a week.
/// Uses Windows' built-in curl, so no TLS stack is compiled into Savepoint.
pub fn update(data_dir: &Path) -> io::Result<()> {
    let files = Files::new(data_dir);
    let fresh = fs::metadata(&files.yaml)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < MAX_AGE);
    if fresh {
        return Ok(());
    }

    let part = data_dir.join("manifest.yaml.part");
    let headers = data_dir.join("manifest.headers");
    let curl =
        PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\curl.exe");
    let mut cmd = Command::new(curl);
    cmd.args(["-sSL", "--compressed", "--max-time", "120", "-w", "%{http_code}", "-o"])
        .arg(&part)
        .arg("-D")
        .arg(&headers)
        .creation_flags(CREATE_NO_WINDOW);
    if files.yaml.exists()
        && let Ok(etag) = fs::read_to_string(&files.etag)
    {
        cmd.arg("-H").arg(format!("If-None-Match: {}", etag.trim()));
    }
    let output = cmd.arg(URL).output()?;
    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let result = match code.as_str() {
        "304" => {
            // Unchanged upstream: just mark the cached copy as fresh again.
            fs::File::options().write(true).open(&files.yaml)?.set_modified(SystemTime::now())
        }
        "200" => {
            fs::rename(&part, &files.yaml)?;
            let _ = fs::remove_file(&files.index);
            if let Some(etag) = read_etag(&headers) {
                let _ = fs::write(&files.etag, etag);
            }
            Ok(())
        }
        _ => Err(io::Error::other(format!(
            "manifest download failed (HTTP {code}): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    };
    let _ = fs::remove_file(&part);
    let _ = fs::remove_file(&headers);
    result
}

fn read_etag(headers: &Path) -> Option<String> {
    let text = fs::read_to_string(headers).ok()?;
    // With redirects there can be several header blocks; the last one wins.
    text.lines()
        .rev()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("etag"))
        .map(|(_, v)| v.trim().to_string())
}

/// Returns the compact index, rebuilding it from the YAML when needed.
pub fn load_index(data_dir: &Path) -> io::Result<Vec<Entry>> {
    let files = Files::new(data_dir);
    let _ = fs::remove_file(data_dir.join("index.json"));
    let _ = fs::remove_file(data_dir.join("index-v2.json"));
    let yaml_time = fs::metadata(&files.yaml).and_then(|m| m.modified())?;
    let index_ok = fs::metadata(&files.index).and_then(|m| m.modified()).is_ok_and(|t| t >= yaml_time);
    if index_ok
        && let Ok(bytes) = fs::read(&files.index)
        && let Ok(entries) = serde_json::from_slice(&bytes)
    {
        return Ok(entries);
    }

    let entries = parse_yaml(&fs::read(&files.yaml)?)?;
    fsutil::write_atomic(&files.index, &serde_json::to_vec(&entries)?)?;
    Ok(entries)
}

fn parse_yaml(bytes: &[u8]) -> io::Result<Vec<Entry>> {
    let raw: HashMap<String, RawGame> = serde_yaml::from_slice(bytes).map_err(io::Error::other)?;
    let mut entries = Vec::new();
    for (name, game) in raw {
        if game.alias.is_some() {
            continue;
        }
        let mut files: Vec<String> = game
            .files
            .into_iter()
            .filter(|(path, info)| applies_to_windows(path, info.as_ref()))
            .map(|(path, _)| path)
            .collect();
        if files.is_empty() {
            continue;
        }
        files.sort();
        let mut install_dirs: Vec<String> = game.install_dir.into_keys().collect();
        install_dirs.sort();
        entries.push(Entry {
            name,
            files,
            install_dirs,
            steam_id: game.steam.and_then(|s| s.id),
            gog_id: game.gog.and_then(|g| g.id),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn applies_to_windows(path: &str, info: Option<&RawFile>) -> bool {
    if path.contains("<xdg") || path.contains("<reg") || path.starts_with("<home>/.") {
        return false;
    }
    // Settings-only paths (tag "config") are not saves. Untagged paths are.
    if info.is_some_and(|i| !i.tags.is_empty() && !i.tags.iter().any(|t| t == "save")) {
        return false;
    }
    match info {
        Some(info) if !info.when.is_empty() => {
            info.when.iter().any(|w| w.os.as_deref().is_none_or(|os| os == "windows"))
        }
        _ => true,
    }
}
