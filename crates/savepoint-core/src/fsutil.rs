use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Writes the whole file or nothing: a crash mid-write leaves the old file intact.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)
}

pub fn mtime_nanos(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0)
}

pub fn mtime_unix(path: &Path) -> Option<i64> {
    let meta = fs::metadata(path).ok()?;
    Some((mtime_nanos(&meta) / 1_000_000_000) as i64)
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Copies `src` to `dst` in one pass, returning the hash of what was copied
/// and carrying over the modification time.
pub fn copy_hashed(src: &Path, dst: &Path) -> io::Result<String> {
    let mut input = File::open(src)?;
    let modified = input.metadata()?.modified()?;
    let mut output = File::create(dst)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
    }
    output.set_modified(modified)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Turns a game name into something safe to use as a folder name.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() { "_".to_string() } else { trimmed.to_string() }
}
